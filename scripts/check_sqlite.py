#!/usr/bin/env python3
"""Execute SQL-engine tests against the shipped migration, without Cargo or network.

These checks do NOT compile Rust, invoke the application's writer/ACL implementation,
qualify native IPC, or establish end-to-end release acceptance. Rust tests are separate.
Only temporary databases created by this script are mutated.
"""
from __future__ import annotations

import json
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = (ROOT / "migrations/0001_authority.sql").read_text(encoding="utf-8")


def static_sql() -> list[tuple[str, str]]:
    statements = []
    for path in sorted((ROOT / "src").rglob("*.rs")):
        if "/acquisition/code/" in path.as_posix():
            continue
        for match in re.finditer(r'"(?:\\.|[^"\\])*"', path.read_text(encoding="utf-8")):
            try:
                value = json.loads(match.group())
            except (ValueError, json.JSONDecodeError):
                continue
            if re.match(r"^(SELECT|INSERT|UPDATE|DELETE|PRAGMA)\s", value):
                if ";" not in value.strip().rstrip(";"):
                    statements.append((str(path.relative_to(ROOT)), value))
    return statements


class AuthoritySQL(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(prefix="huda-sql-check-")
        self.path = Path(self.scratch.name) / "authority.db"
        self.db = sqlite3.connect(self.path)
        self.db.execute("PRAGMA foreign_keys=ON")
        self.db.executescript(SCHEMA)
        self.db.execute("INSERT INTO relation_types VALUES ('core:DEPENDS_ON',1,'{}',1)")
        for name in ("a", "b", "c"):
            self.db.execute("INSERT INTO entities VALUES (?, 'demo', 'concept', ?,1)", (name, name))
        for name in ("r1", "r2"):
            self.db.execute("INSERT INTO relations VALUES (?,?,1,'core:DEPENDS_ON',1,'{}',1)", (name, name))
        self.db.commit()

    def tearDown(self) -> None:
        self.db.close()
        self.scratch.cleanup()

    def assertion(self, assertion_id: str, *, source: str | None = None,
                  relation: str = "r1", scope: str = "local", facet: str = "facet-a") -> None:
        if source is not None:
            self.db.execute("INSERT OR IGNORE INTO sources VALUES (?,'owner',?,'{}',0,NULL,0)", (source, scope))
        self.db.execute(
            "INSERT INTO relation_assertions VALUES (?,?, 'owner', ?, ?, ?, ?, ?,1,'active')",
            (assertion_id, relation, scope, source, assertion_id,
             facet if source else None, "fixture-v1" if source else None),
        )
        origin = json.dumps({"kind": "structured_import" if source else "user", "actor": "owner", "operation_id": assertion_id})
        self.db.execute("INSERT INTO assertion_versions VALUES (?,?,1,1,'active','asserted',?,'[]')",
                        (assertion_id + "-v1", assertion_id, origin))
        self.db.commit()

    def test_schema_integrity_and_foreign_keys(self) -> None:
        self.assertEqual(self.db.execute("PRAGMA integrity_check").fetchone()[0], "ok")
        self.assertEqual(self.db.execute("PRAGMA foreign_key_check").fetchall(), [])

    def test_no_code_index_authority_tables(self) -> None:
        tables = {row[0] for row in self.db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertTrue({"relations", "relation_assertions", "assertion_versions", "source_extraction_scopes"} <= tables)
        self.assertFalse({"files", "nodes", "edges", "claims", "observations"} & tables)

    def test_evidence_free_direct_assertion(self) -> None:
        self.assertion("manual")
        self.assertEqual(self.db.execute("SELECT source_key FROM relation_assertions").fetchone()[0], None)
        self.assertEqual(self.db.execute("SELECT evidence_json FROM assertion_versions").fetchone()[0], "[]")
        self.assertEqual(self.db.execute("SELECT count(*) FROM sources").fetchone()[0], 0)

    def test_nary_and_literal_roles_are_not_binary_edges(self) -> None:
        self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','subject',0,'a',NULL)")
        self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','object',0,'b',NULL)")
        self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','context',0,'c',NULL)")
        self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','amount',0,NULL,?)", (json.dumps({"type": "integer", "value": 3}),))
        self.assertEqual(self.db.execute("SELECT count(*) FROM relation_role_bindings WHERE relation_id='r1'").fetchone()[0], 4)
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','invalid',0,'a','{}')")

    def test_unresolved_entity_role_is_rejected(self) -> None:
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO relation_role_bindings VALUES ('r1','unknown',0,'absent',NULL)")

    def test_persistent_inference_is_rejected_by_sql(self) -> None:
        self.assertion("manual")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO assertion_versions VALUES ('illegal','manual',2,2,'active','inferred',?,'[]')", (json.dumps({"kind": "user", "actor": "owner", "operation_id": "op"}),))

    def test_origin_object_and_required_technical_fields(self) -> None:
        self.assertion("manual")
        for origin in (None, "[]", "{}", '{"kind":"user","actor":"owner"}'):
            with self.subTest(origin=origin), self.assertRaises(sqlite3.IntegrityError):
                self.db.execute("INSERT INTO assertion_versions VALUES ('bad','manual',2,2,'active','asserted',?,'[]')", (origin,))

    def test_evidence_is_optional_but_must_be_an_array(self) -> None:
        self.assertion("manual")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO assertion_versions VALUES ('bad','manual',2,2,'active','asserted',?,'{}')", (json.dumps({"kind": "user", "actor": "owner", "operation_id": "op"}),))

    def test_immutable_assertion_history_and_identity(self) -> None:
        self.assertion("manual")
        for statement in ("UPDATE assertion_versions SET status='withdrawn'", "DELETE FROM assertion_versions", "UPDATE relation_assertions SET actor='other'", "DELETE FROM relation_assertions", "UPDATE relations SET canonical_key='changed'"):
            with self.subTest(statement=statement), self.assertRaises(sqlite3.IntegrityError):
                self.db.execute(statement)

    def test_current_head_cannot_point_to_a_missing_version(self) -> None:
        self.assertion("manual")
        self.db.execute("UPDATE relation_assertions SET current_version=99 WHERE assertion_id='manual'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.commit()
        self.db.rollback()
        self.assertEqual(self.db.execute("SELECT current_version FROM relation_assertions").fetchone()[0], 1)

    def test_independent_assertions_survive_scoped_sql_withdrawal(self) -> None:
        self.assertion("manual")
        self.assertion("import-a", source="source-a")
        self.assertion("import-b", source="source-b")
        origin = self.db.execute("SELECT origin_json FROM assertion_versions WHERE assertion_id='import-a'").fetchone()[0]
        self.db.execute("INSERT INTO assertion_versions VALUES ('import-a-v2','import-a',2,2,'withdrawn','asserted',?,'[]')", (origin,))
        self.db.execute("UPDATE relation_assertions SET current_version=2,current_status='withdrawn' WHERE source_key='source-a'")
        self.db.commit()
        active = {row[0] for row in self.db.execute("SELECT assertion_id FROM relation_assertions WHERE current_status='active'")}
        self.assertEqual(active, {"manual", "import-b"})

    def test_actual_reconciliation_selector_is_exact_scope(self) -> None:
        self.assertion("manual")
        self.assertion("facet-a", source="source-a", facet="facet-a")
        self.assertion("facet-b", source="source-a", facet="facet-b")
        self.assertion("different-source", source="source-b", facet="facet-a")
        statement = next(query for file, query in static_sql() if file.endswith("publish.rs") and query.startswith("SELECT assertion_id,relation_id FROM relation_assertions WHERE source_key=?1 AND extraction_scope_key=?2"))
        self.assertEqual(self.db.execute(statement, ("source-a", "facet-a")).fetchall(), [("facet-a", "r1")])

    def test_private_assertions_do_not_change_scoped_query_rows(self) -> None:
        self.assertion("public", scope="local")
        query = "SELECT assertion_id FROM relation_assertions WHERE relation_id=? AND scope IN(SELECT value FROM json_each(?)) ORDER BY assertion_id"
        before = self.db.execute(query, ("r1", '["local"]')).fetchall()
        self.assertion("private", scope="private")
        self.assertEqual(before, self.db.execute(query, ("r1", '["local"]')).fetchall())
        self.assertEqual(self.db.execute(query, ("r1", "[]")).fetchall(), [])

    def test_history_cannot_bind_another_relations_assertion(self) -> None:
        self.assertion("first", relation="r1")
        self.assertion("second", relation="r2")
        self.db.execute("INSERT INTO relation_versions VALUES ('rv1','r1',1,'fixture')")
        self.db.execute("INSERT INTO relation_version_assertions VALUES ('rv1','first-v1')")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("INSERT INTO relation_version_assertions VALUES ('rv1','second-v1')")

    def test_registry_versions_are_immutable(self) -> None:
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("UPDATE relation_types SET definition_json='{\"changed\":true}'")
        self.db.execute("INSERT INTO relation_types VALUES ('core:DEPENDS_ON',2,'{}',2)")
        self.assertEqual(self.db.execute("SELECT count(*) FROM relation_types").fetchone()[0], 2)

    def test_restart_job_recovery_preserves_commit_evidence(self) -> None:
        report = json.dumps({"committed_sources": [{"source_key": "a", "generation": 1}]})
        self.db.execute("INSERT INTO jobs VALUES ('j','owner','local','source_ingestion','running','{}',0,?,1,1,1)", (report,))
        statement = next(query for file, query in static_sql() if file.endswith("storage/mod.rs") and query.startswith("UPDATE jobs SET state='failed'"))
        self.db.execute(statement, (2,))
        actual = json.loads(self.db.execute("SELECT report_json FROM jobs").fetchone()[0])
        self.assertEqual(actual["committed_sources"], [{"source_key": "a", "generation": 1}])
        self.assertTrue(actual["publication_may_have_committed"])

    def test_sqlite_backup_and_strict_read_only_open(self) -> None:
        self.assertion("manual")
        backup = Path(self.scratch.name) / "backup.db"
        with sqlite3.connect(backup) as target:
            self.db.backup(target)
        reader = sqlite3.connect(backup.as_uri() + "?mode=ro", uri=True)
        try:
            reader.execute("PRAGMA query_only=ON")
            self.assertEqual(reader.execute("SELECT count(*) FROM relation_assertions").fetchone()[0], 1)
            with self.assertRaises(sqlite3.OperationalError):
                reader.execute("UPDATE system_meta SET tx_tick=99")
            self.assertEqual(reader.execute("PRAGMA integrity_check").fetchone()[0], "ok")
        finally:
            reader.close()

    def test_sqlite_wal_rollback_after_uncommitted_process_exit(self) -> None:
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.close()
        program = "import sqlite3,sys,os; c=sqlite3.connect(sys.argv[1]); c.execute('BEGIN IMMEDIATE'); c.execute(\"INSERT INTO entities VALUES ('crash','demo','concept','crash',3)\"); os._exit(17)"
        process = subprocess.run([sys.executable, "-c", program, str(self.path)], check=False, timeout=5, capture_output=True)
        self.assertEqual(process.returncode, 17)
        self.db = sqlite3.connect(self.path)
        self.assertEqual(self.db.execute("SELECT count(*) FROM entities WHERE entity_id='crash'").fetchone()[0], 0)
        self.assertEqual(self.db.execute("PRAGMA integrity_check").fetchone()[0], "ok")

    def test_all_static_rust_sql_statements_compile_against_migration(self) -> None:
        statements = static_sql()
        self.assertGreaterEqual(len(statements), 100)
        for path, statement in statements:
            count = max([int(value) for value in re.findall(r"\?(\d+)", statement)] or [0])
            with self.subTest(file=path, statement=statement):
                self.db.execute("EXPLAIN " + statement, [None] * count)


if __name__ == "__main__":
    print(f"Python SQL-engine checks only | SQLite {sqlite3.sqlite_version} | {len(static_sql())} static SQL statements", flush=True)
    unittest.main(verbosity=2)
