#!/usr/bin/env python3
"""Explicit native client gold runner. Requires a compiled helper and initialized test profile."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import subprocess
import time
import uuid


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--huda", default="huda")
    parser.add_argument("--profile", type=Path)
    args = parser.parse_args()
    prefix = [args.huda]
    if args.profile:
        prefix += ["--profile", str(args.profile.resolve())]

    def call(operation: str, value: dict) -> dict:
        result = subprocess.run(prefix + ["call", operation], input=json.dumps(value), text=True,
                                capture_output=True, timeout=130, check=False)
        if result.returncode:
            raise RuntimeError(f"{operation} failed: {result.stderr.strip()}")
        return json.loads(result.stdout)

    def epoch() -> int:
        return call("status", {})["epochs"]["policy"]

    # An explicit new origin key avoids treating reruns as an unverified old import revision.
    run = uuid.uuid4().hex
    ids = []
    for key, name in (("project-a", "Project A"), ("service-b", "Service B")):
        result = call("upsert_entity", {"entity": {"namespace": "demo", "kind": "concept", "canonical_key": key,
                                                  "name": name, "aliases": []},
                                         "scope": "local", "expected_policy_epoch": epoch()})
        ids.append(result["entity_id"])
    relation = {"relation_type": "core:DEPENDS_ON@1", "roles": {
        "dependent": [{"kind": "entity", "entity_id": ids[0]}],
        "dependency": [{"kind": "entity", "entity_id": ids[1]}]},
        "qualifiers": {"environment": {"type": "string", "value": "production"}}}
    direct = call("assert_relation", {"relation": relation, "scope": "local", "idempotency_key": run,
                                       "expected_policy_epoch": epoch()})
    source = "demo-" + run
    call("admit_source", {"policy": {"source_key": source, "scope": "local", "allowed_actors": ["owner"],
        "allowed_types": ["core:DEPENDS_ON@1"], "entity_namespaces": ["demo"], "auto_admit_structured": True,
        "auto_admit_semantic": False}, "expected_policy_epoch": epoch()})
    job = call("ingest_source", {"source_key": source, "revision": "1", "expected_generation": 0,
        "extraction_scope_key": "all", "coverage_complete_for_scope": True, "idempotency_key": run,
        "expected_policy_epoch": epoch(), "payload": {"format": "relations", "data": {"entities": [],
        "assertions": [{"anchor": "dependency", "relation": relation}]}}})
    deadline = time.monotonic() + 30
    while job["state"] in ("queued", "running"):
        if time.monotonic() > deadline:
            raise RuntimeError("Import did not finish; inspect its actual job state before any retry")
        time.sleep(0.05)
        job = call("get_job", {"job_id": job["job_id"]})
    if job["state"] != "completed":
        raise RuntimeError(f"Import did not complete: {job['state']} {job.get('report')}")
    before = call("get_relation", {"relation_id": direct["relation_id"]})
    assert any(a["origin"]["source_key"] == source for a in before["assertions"])
    assert all(a["evidence_refs"] == [] for a in before["assertions"])
    generation = call("get_source", {"source_key": source})["generation"]
    call("withdraw_source", {"source_key": source, "expected_generation": generation,
                             "expected_policy_epoch": epoch()})
    after = call("get_relation", {"relation_id": direct["relation_id"]})
    assert any(a["assertion_id"] == direct["assertion_id"] for a in after["assertions"])
    assert not any(a["origin"]["source_key"] == source for a in after["assertions"])
    answer = call("ask_huda", {"question": "What does Project A depend on in production?", "expansion_mode": "off"})
    print(json.dumps({"relation_id": direct["relation_id"], "independent_json_withdrawal": "passed",
                      "evidence_required": False, "ask_status": answer["status"],
                      "note": "model_unavailable is correct without an explicitly configured local model"}, indent=2))


if __name__ == "__main__":
    main()
