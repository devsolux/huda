//! Versioned, deterministic type validation; no inferred global ontology mappings.
use super::condition::Condition;
use super::model::*;
use crate::error::{require, Error, Result};
use crate::storage::{audit, tick, Store};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

pub fn validate_type(ty: &RelationType) -> Result<()> {
    let Some((namespace, name)) = ty.name.split_once(':') else {
        return Err(Error::invalid("Relation type needs a namespace"));
    };
    identifier(namespace)?;
    identifier(name)?;
    require(
        !name.contains(':') && !name.contains('@') && ty.version > 0,
        "INVALID_INPUT",
        "Invalid type name/version",
    )?;
    text(&ty.description, 4096)?;
    require(
        (2..=16).contains(&ty.roles.len())
            && ty.qualifiers.len() <= 16
            && ty.traversals.len() <= 32,
        "LIMIT_EXCEEDED",
        "Type exceeds role/qualifier/traversal limits",
    )?;
    for (role, spec) in &ty.roles {
        identifier(role)?;
        require(
            spec.min <= spec.max && spec.max > 0 && spec.max <= 16,
            "INVALID_INPUT",
            "Invalid role cardinality",
        )?;
        require(
            !spec.entity_kinds.is_empty() || !spec.literal_types.is_empty(),
            "INVALID_INPUT",
            "Role needs explicit value types",
        )?;
        for kind in &spec.entity_kinds {
            if kind != "*" {
                identifier(kind)?;
            }
        }
        for kind in &spec.literal_types {
            literal_type(kind)?;
        }
    }
    for (qualifier, kind) in &ty.qualifiers {
        identifier(qualifier)?;
        literal_type(kind)?;
    }
    for traversal in &ty.traversals {
        require(
            ty.roles.contains_key(&traversal.from_role)
                && ty.roles.contains_key(&traversal.to_role)
                && traversal.from_role != traversal.to_role,
            "INVALID_INPUT",
            "Traversal must name distinct declared roles",
        )?;
    }
    Ok(())
}
fn literal_type(kind: &str) -> Result<()> {
    require(
        matches!(
            kind,
            "string" | "integer" | "boolean" | "decimal" | "timestamp" | "quantity"
        ),
        "INVALID_INPUT",
        "Unsupported literal datatype",
    )
}
pub(crate) fn load_type(conn: &Connection, key: &str) -> Result<RelationType> {
    let (name, version) = key
        .rsplit_once('@')
        .ok_or_else(|| Error::invalid("Type must use namespace:name@version"))?;
    let version: u32 = version
        .parse()
        .map_err(|_| Error::invalid("Invalid type version"))?;
    let body: Option<String> = conn
        .query_row(
            "SELECT definition_json FROM relation_types WHERE name=?1 AND version=?2",
            params![name, version],
            |r| r.get(0),
        )
        .optional()?;
    serde_json::from_str(&body.ok_or_else(|| {
        Error::new(
            "TYPE_UNKNOWN",
            "Register this exact type version before publishing",
        )
    })?)
    .map_err(Into::into)
}
pub(crate) fn normalize_relation(
    conn: &Connection,
    caller: &Caller,
    input: &RelationInput,
    scope: &str,
) -> Result<(RelationInput, String)> {
    let ty = load_type(conn, &input.relation_type)?;
    let mut relation = input.clone();
    require(
        relation.roles.keys().all(|k| ty.roles.contains_key(k)),
        "INVALID_INPUT",
        "Undeclared role",
    )?;
    for (name, spec) in &ty.roles {
        let values = relation.roles.entry(name.clone()).or_default();
        require(
            values.len() >= usize::from(spec.min) && values.len() <= usize::from(spec.max),
            "INVALID_INPUT",
            "Role cardinality mismatch",
        )?;
        for value in values.iter() {
            match value {
                RoleValue::Entity { entity_id } => {
                    let kind:Option<String>=conn.query_row("SELECT e.kind FROM entities e WHERE e.entity_id=?1 AND EXISTS(SELECT 1 FROM entity_labels l WHERE l.entity_id=e.entity_id AND l.scope=?2)",params![entity_id,scope],|r|r.get(0)).optional()?;
                    let kind = kind.ok_or_else(Error::not_found)?;
                    require(
                        caller.read(scope)
                            && (spec.entity_kinds.contains("*")
                                || spec.entity_kinds.contains(&kind)),
                        "INVALID_INPUT",
                        "Role entity kind or visibility mismatch",
                    )?;
                }
                RoleValue::Literal { literal } => {
                    literal.validate()?;
                    require(
                        spec.literal_types.contains(literal.type_name()),
                        "INVALID_INPUT",
                        "Role literal datatype mismatch",
                    )?;
                }
            }
        }
        if !spec.ordered {
            values.sort();
            values.dedup();
        }
        require(
            values.len() >= usize::from(spec.min),
            "INVALID_INPUT",
            "Role contains too few distinct values",
        )?;
    }
    for (name, value) in &relation.qualifiers {
        value.validate()?;
        require(
            ty.qualifiers
                .get(name)
                .is_some_and(|kind| kind == value.type_name()),
            "INVALID_INPUT",
            "Qualifier is undeclared or has the wrong datatype",
        )?;
    }
    require(
        ty.allow_negation || relation.polarity == Polarity::Positive,
        "INVALID_INPUT",
        "Type does not allow negation",
    )?;
    require(
        ty.allow_conditions || relation.condition == Condition::True,
        "INVALID_INPUT",
        "Type does not allow conditions",
    )?;
    require(
        ty.allow_valid_time || relation.valid_time.is_none(),
        "INVALID_INPUT",
        "Type does not allow valid time",
    )?;
    if let Some(time) = &relation.valid_time {
        require(
            time.start.is_some() || time.end.is_some(),
            "INVALID_INPUT",
            "Use null for undetermined valid time",
        )?;
        if let (Some(start), Some(end)) = (time.start, time.end) {
            require(
                start < end,
                "INVALID_INPUT",
                "Valid-time interval must have start < end",
            )?;
        }
    }
    relation.condition.normalize()?;
    let id = stable_id("rel", &(1u32, &relation))?;
    Ok((relation, id))
}
impl Store {
    pub(crate) fn bootstrap_types(&mut self) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: u64 = tx.query_row("SELECT count(*) FROM relation_types", [], |r| r.get(0))?;
        if count == 0 {
            for ty in starter_types() {
                validate_type(&ty)?;
                tx.execute("INSERT INTO relation_types(name,version,definition_json,tx_tick) VALUES(?1,?2,?3,0)",params![ty.name,ty.version,serde_json::to_string(&ty)?])?;
            }
            tx.execute(
                "UPDATE system_meta SET registry_epoch=1 WHERE singleton=1",
                [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn list_types(&self, caller: &Caller) -> Result<Vec<RelationType>> {
        caller.require("read")?;
        let mut stmt = self
            .conn
            .prepare("SELECT definition_json FROM relation_types ORDER BY name,version")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.iter()
            .map(|s| serde_json::from_str(s).map_err(Into::into))
            .collect()
    }
    pub fn register_type(
        &mut self,
        caller: &Caller,
        req: RegisterTypeRequest,
    ) -> Result<RelationType> {
        caller.require("registry")?;
        validate_type(&req.relation_type)?;
        // Provider names require the separately checked mechanical registration path.
        require(
            !req.relation_type.name.starts_with("wikidata:"),
            "PERMISSION_DENIED",
            "Provider types use the qualified provider registry path",
        )?;
        let tx = self.begin(caller, req.expected_policy_epoch)?;
        let epoch: u64 = tx.query_row(
            "SELECT registry_epoch FROM system_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        require(
            epoch == req.expected_registry_epoch,
            "VERSION_CONFLICT",
            "Registry changed",
        )?;
        let latest: u32 = tx.query_row(
            "SELECT coalesce(max(version),0) FROM relation_types WHERE name=?1",
            [&req.relation_type.name],
            |r| r.get(0),
        )?;
        require(
            req.relation_type.version == latest + 1,
            "VERSION_CONFLICT",
            "Type versions must append consecutively",
        )?;
        let t = tick(&tx)?;
        tx.execute(
            "INSERT INTO relation_types(name,version,definition_json,tx_tick) VALUES(?1,?2,?3,?4)",
            params![
                req.relation_type.name,
                req.relation_type.version,
                serde_json::to_string(&req.relation_type)?,
                t
            ],
        )?;
        tx.execute("UPDATE system_meta SET registry_epoch=registry_epoch+1,policy_epoch=policy_epoch+1 WHERE singleton=1",[])?;
        audit(&tx, t, caller, "register_type", &req.relation_type.key())?;
        tx.commit()?;
        Ok(req.relation_type)
    }
}
pub fn starter_types() -> Vec<RelationType> {
    fn entity() -> RoleSpec {
        RoleSpec {
            entity_kinds: BTreeSet::from(["*".into()]),
            literal_types: BTreeSet::new(),
            min: 1,
            max: 1,
            ordered: false,
        }
    }
    let mut types = Vec::new();
    for (name, from, to) in [
        ("IS_A", "instance", "class"),
        ("PART_OF", "part", "whole"),
        ("DEPENDS_ON", "dependent", "dependency"),
        ("PRECEDES", "earlier", "later"),
        ("LOCATED_IN", "located", "location"),
    ] {
        types.push(RelationType {
            name: format!("core:{name}"),
            version: 1,
            description: format!(
                "Explicit {from} to {to} relation. Paths are not automatically new facts."
            ),
            roles: BTreeMap::from([(from.into(), entity()), (to.into(), entity())]),
            qualifiers: BTreeMap::from([("environment".into(), "string".into())]),
            traversals: vec![Traversal {
                from_role: from.into(),
                to_role: to.into(),
                bidirectional: false,
            }],
            allow_conditions: true,
            allow_negation: true,
            allow_valid_time: true,
        });
    }
    types.push(RelationType {
        name: "core:HAS_PROPERTY".into(),
        version: 1,
        description: "A named, typed property value of a subject.".into(),
        roles: BTreeMap::from([
            ("subject".into(), entity()),
            (
                "property".into(),
                RoleSpec {
                    entity_kinds: BTreeSet::new(),
                    literal_types: BTreeSet::from(["string".into()]),
                    min: 1,
                    max: 1,
                    ordered: false,
                },
            ),
            (
                "value".into(),
                RoleSpec {
                    entity_kinds: BTreeSet::from(["*".into()]),
                    literal_types: [
                        "string",
                        "integer",
                        "boolean",
                        "decimal",
                        "timestamp",
                        "quantity",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    min: 1,
                    max: 1,
                    ordered: false,
                },
            ),
        ]),
        qualifiers: BTreeMap::new(),
        traversals: Vec::new(),
        allow_conditions: true,
        allow_negation: true,
        allow_valid_time: true,
    });
    types
}
