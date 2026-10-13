-- Huda authority schema 1. Never run against an unidentified/code-index database.
-- Application/user versions are set in the same bootstrap transaction by Store.
CREATE TABLE system_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    tx_tick INTEGER NOT NULL CHECK(tx_tick>=0),
    policy_epoch INTEGER NOT NULL CHECK(policy_epoch>=0),
    registry_epoch INTEGER NOT NULL CHECK(registry_epoch>=0),
    semantic_epoch INTEGER NOT NULL CHECK(semantic_epoch=1),
    offline INTEGER NOT NULL CHECK(offline IN (0,1))
) STRICT;
INSERT INTO system_meta VALUES(1,0,0,0,1,1);

CREATE TABLE principals (
    actor TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    read_scopes_json TEXT NOT NULL CHECK(json_valid(read_scopes_json)),
    write_scopes_json TEXT NOT NULL CHECK(json_valid(write_scopes_json)),
    permissions_json TEXT NOT NULL CHECK(json_valid(permissions_json)),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    version INTEGER NOT NULL CHECK(version>=1)
) STRICT;

CREATE TABLE entities (
    entity_id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    kind TEXT NOT NULL,
    canonical_key TEXT NOT NULL,
    created_tick INTEGER NOT NULL,
    UNIQUE(namespace,kind,canonical_key)
) STRICT;
CREATE TABLE entity_labels (
    entity_id TEXT NOT NULL REFERENCES entities(entity_id),
    scope TEXT NOT NULL,
    label TEXT NOT NULL,
    actor TEXT NOT NULL,
    tx_tick INTEGER NOT NULL,
    PRIMARY KEY(entity_id,scope,label)
) STRICT;
CREATE INDEX entity_labels_lookup ON entity_labels(scope,label);

CREATE TABLE relation_types (
    name TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version>=1),
    definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
    tx_tick INTEGER NOT NULL,
    PRIMARY KEY(name,version)
) STRICT;
CREATE TABLE relations (
    relation_id TEXT PRIMARY KEY,
    canonical_key TEXT NOT NULL UNIQUE,
    canonicalizer_version INTEGER NOT NULL CHECK(canonicalizer_version=1),
    type_name TEXT NOT NULL,
    type_version INTEGER NOT NULL,
    relation_json TEXT NOT NULL CHECK(json_valid(relation_json)),
    created_tick INTEGER NOT NULL,
    FOREIGN KEY(type_name,type_version) REFERENCES relation_types(name,version)
) STRICT;
CREATE TABLE relation_role_bindings (
    relation_id TEXT NOT NULL REFERENCES relations(relation_id),
    role TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    entity_id TEXT REFERENCES entities(entity_id),
    literal_json TEXT CHECK(literal_json IS NULL OR json_valid(literal_json)),
    PRIMARY KEY(relation_id,role,ordinal),
    CHECK((entity_id IS NOT NULL AND literal_json IS NULL) OR (entity_id IS NULL AND literal_json IS NOT NULL))
) STRICT;
CREATE INDEX role_entity_lookup ON relation_role_bindings(entity_id,relation_id);
CREATE INDEX relation_type_lookup ON relations(type_name,type_version);

CREATE TABLE sources (
    source_key TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    scope TEXT NOT NULL,
    policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
    generation INTEGER NOT NULL CHECK(generation>=0),
    current_revision TEXT,
    withdrawn INTEGER NOT NULL DEFAULT 0 CHECK(withdrawn IN (0,1))
) STRICT;
CREATE TABLE source_revisions (
    source_key TEXT NOT NULL REFERENCES sources(source_key),
    generation INTEGER NOT NULL CHECK(generation>=1),
    revision TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    capture_json TEXT NOT NULL CHECK(json_valid(capture_json)),
    tx_tick INTEGER NOT NULL,
    PRIMARY KEY(source_key,generation)
) STRICT;
CREATE TABLE source_extraction_scopes (
    source_key TEXT NOT NULL REFERENCES sources(source_key),
    scope_key TEXT NOT NULL,
    generation INTEGER NOT NULL,
    revision TEXT NOT NULL,
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    extractor_digest TEXT NOT NULL,
    PRIMARY KEY(source_key,scope_key)
) STRICT;

CREATE TABLE relation_assertions (
    assertion_id TEXT PRIMARY KEY,
    relation_id TEXT NOT NULL REFERENCES relations(relation_id),
    actor TEXT NOT NULL,
    scope TEXT NOT NULL,
    source_key TEXT REFERENCES sources(source_key),
    origin_assertion_key TEXT NOT NULL,
    extraction_scope_key TEXT,
    extractor_digest TEXT,
    current_version INTEGER NOT NULL CHECK(current_version>=1),
    current_status TEXT NOT NULL CHECK(current_status IN ('active','proposed','withdrawn','rejected','superseded')),
    UNIQUE(source_key,origin_assertion_key,relation_id,extractor_digest),
    CHECK((source_key IS NULL AND extraction_scope_key IS NULL AND extractor_digest IS NULL)
       OR (source_key IS NOT NULL AND extraction_scope_key IS NOT NULL AND extractor_digest IS NOT NULL)),
    FOREIGN KEY(assertion_id,current_version,current_status)
      REFERENCES assertion_versions(assertion_id,version,status) DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE INDEX assertion_acl ON relation_assertions(scope,current_status,relation_id);
CREATE INDEX assertion_source_scope ON relation_assertions(source_key,extraction_scope_key,current_status);
CREATE TABLE assertion_versions (
    assertion_version_id TEXT PRIMARY KEY,
    assertion_id TEXT NOT NULL REFERENCES relation_assertions(assertion_id),
    version INTEGER NOT NULL,
    tx_tick INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active','proposed','withdrawn','rejected','superseded')),
    epistemic_kind TEXT NOT NULL CHECK(epistemic_kind IN ('asserted','extracted','hypothetical')),
    origin_json TEXT NOT NULL CHECK(json_valid(origin_json)
      AND json_type(origin_json) IS 'object'
      AND json_type(origin_json,'$.kind') IS 'text'
      AND json_type(origin_json,'$.actor') IS 'text'
      AND json_type(origin_json,'$.operation_id') IS 'text'
      AND length(json_extract(origin_json,'$.kind'))>0
      AND length(json_extract(origin_json,'$.actor'))>0
      AND length(json_extract(origin_json,'$.operation_id'))>0),
    evidence_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence_json) AND json_type(evidence_json) IS 'array'),
    UNIQUE(assertion_id,version),
    UNIQUE(assertion_id,version,status)
) STRICT;
CREATE INDEX assertion_version_snapshot ON assertion_versions(assertion_id,tx_tick DESC);
CREATE TABLE relation_versions (
    relation_version_id TEXT PRIMARY KEY,
    relation_id TEXT NOT NULL REFERENCES relations(relation_id),
    tx_tick INTEGER NOT NULL,
    active_set_hash TEXT NOT NULL,
    UNIQUE(relation_id,tx_tick)
) STRICT;
CREATE TABLE relation_version_assertions (
    relation_version_id TEXT NOT NULL REFERENCES relation_versions(relation_version_id),
    assertion_version_id TEXT NOT NULL REFERENCES assertion_versions(assertion_version_id),
    PRIMARY KEY(relation_version_id,assertion_version_id)
) STRICT;

CREATE TABLE relation_proposals (
    proposal_id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    scope TEXT NOT NULL,
    source_key TEXT REFERENCES sources(source_key),
    reason TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    tx_tick INTEGER NOT NULL
) STRICT;
CREATE TABLE write_requests (
    actor TEXT NOT NULL,
    request_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    response_json TEXT NOT NULL CHECK(json_valid(response_json)),
    PRIMARY KEY(actor,request_key)
) STRICT;
CREATE TABLE write_events (
    event_id TEXT PRIMARY KEY,
    tx_tick INTEGER NOT NULL,
    actor TEXT NOT NULL,
    kind TEXT NOT NULL,
    target_id TEXT NOT NULL
) STRICT;
CREATE TABLE knowledge_provider_policies (
    provider_id TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
    version INTEGER NOT NULL CHECK(version>=1)
) STRICT;
CREATE TABLE jobs (
    job_id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','completed','partial','blocked','cancelled','failed','superseded')),
    goal_json TEXT CHECK(goal_json IS NULL OR json_valid(goal_json)),
    policy_epoch INTEGER NOT NULL,
    report_json TEXT NOT NULL CHECK(json_valid(report_json)),
    version INTEGER NOT NULL CHECK(version>=1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX jobs_actor ON jobs(actor,created_at DESC);
CREATE TABLE bridge_sources (
    bridge_id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    scope TEXT NOT NULL,
    config_json TEXT NOT NULL CHECK(json_valid(config_json)),
    version INTEGER NOT NULL CHECK(version>=1)
) STRICT;

-- The SQL engine also refuses history/identity rewrites, independently of DTO validation.
CREATE TRIGGER immutable_entities_update BEFORE UPDATE ON entities BEGIN SELECT RAISE(ABORT,'immutable entity identity'); END;
CREATE TRIGGER immutable_entities_delete BEFORE DELETE ON entities BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_types_update BEFORE UPDATE ON relation_types BEGIN SELECT RAISE(ABORT,'immutable type version'); END;
CREATE TRIGGER immutable_types_delete BEFORE DELETE ON relation_types BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_relations_update BEFORE UPDATE ON relations BEGIN SELECT RAISE(ABORT,'immutable relation identity'); END;
CREATE TRIGGER immutable_relations_delete BEFORE DELETE ON relations BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_roles_update BEFORE UPDATE ON relation_role_bindings BEGIN SELECT RAISE(ABORT,'immutable role binding'); END;
CREATE TRIGGER immutable_roles_delete BEFORE DELETE ON relation_role_bindings BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_assertion_identity BEFORE UPDATE ON relation_assertions
WHEN NEW.assertion_id!=OLD.assertion_id OR NEW.relation_id!=OLD.relation_id OR NEW.actor!=OLD.actor OR NEW.scope!=OLD.scope
  OR NEW.source_key IS NOT OLD.source_key OR NEW.origin_assertion_key!=OLD.origin_assertion_key
  OR NEW.extraction_scope_key IS NOT OLD.extraction_scope_key OR NEW.extractor_digest IS NOT OLD.extractor_digest
BEGIN SELECT RAISE(ABORT,'immutable assertion ownership'); END;
CREATE TRIGGER immutable_assertion_delete BEFORE DELETE ON relation_assertions BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_av_update BEFORE UPDATE ON assertion_versions BEGIN SELECT RAISE(ABORT,'immutable assertion version'); END;
CREATE TRIGGER immutable_av_delete BEFORE DELETE ON assertion_versions BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_rv_update BEFORE UPDATE ON relation_versions BEGIN SELECT RAISE(ABORT,'immutable relation version'); END;
CREATE TRIGGER immutable_rv_delete BEFORE DELETE ON relation_versions BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_rva_update BEFORE UPDATE ON relation_version_assertions BEGIN SELECT RAISE(ABORT,'immutable historical set'); END;
CREATE TRIGGER immutable_rva_delete BEFORE DELETE ON relation_version_assertions BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_revision_update BEFORE UPDATE ON source_revisions BEGIN SELECT RAISE(ABORT,'immutable source capture'); END;
CREATE TRIGGER immutable_revision_delete BEFORE DELETE ON source_revisions BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;
CREATE TRIGGER immutable_event_update BEFORE UPDATE ON write_events BEGIN SELECT RAISE(ABORT,'immutable audit event'); END;
CREATE TRIGGER immutable_event_delete BEFORE DELETE ON write_events BEGIN SELECT RAISE(ABORT,'erasure is not enabled'); END;

CREATE TRIGGER valid_historical_assertion_insert BEFORE INSERT ON relation_version_assertions
WHEN NOT EXISTS (
  SELECT 1 FROM relation_versions rv
  JOIN relation_assertions a ON a.relation_id=rv.relation_id
  JOIN assertion_versions av ON av.assertion_id=a.assertion_id
  WHERE rv.relation_version_id=NEW.relation_version_id
    AND av.assertion_version_id=NEW.assertion_version_id AND av.status='active'
)
BEGIN SELECT RAISE(ABORT,'historical set must contain active versions of the same relation'); END;
