//! Structured interpretation lives here, not in the network adapter or expansion controller.
use super::admission::{Capture, ExtractedData, StructuredAssertion};
use super::providers::{wikimedia::external_id, PropertyMetadata};
use crate::error::{Error, Result};
use crate::relation::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) fn wikidata(
    capture: &Capture,
    language: &str,
    properties: &[PropertyMetadata],
) -> Result<(ExtractedData, Vec<(String, Value)>)> {
    let item = &capture.payload;
    let qid = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| external_id(id, b'Q'))
        .ok_or_else(|| Error::invalid("Capture is not a Wikidata item"))?;
    let subject = item_entity(qid, label(item, language).unwrap_or(qid));
    let subject_id = subject.id()?;
    let mut entities = BTreeMap::from([(qid.to_owned(), subject)]);
    let mut assertions = Vec::new();
    let mut proposals = Vec::new();
    let claims = item
        .get("claims")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            Error::new(
                "UNSUPPORTED_SCHEMA",
                "Wikidata claims schema is unsupported",
            )
        })?;
    for (pid, statements) in claims.iter().take(128) {
        let Some(statements) = statements.as_array() else {
            continue;
        };
        for statement in statements.iter().take(128) {
            if assertions.len() + proposals.len() >= 1000 {
                return Err(Error::new(
                    "LIMIT_EXCEEDED",
                    "Wikidata statement bound exceeded; no partial enumeration is attested",
                ));
            }
            let mapped = (|| -> Option<StructuredAssertion> {
                let property = properties.iter().find(|p| p.pid == *pid)?;
                if statement.get("rank").and_then(Value::as_str) != Some("normal") {
                    return None;
                }
                // Qualifiers, preferred/deprecated rank, unknown/no value and unsupported types
                // must remain attributed proposals until their semantics are qualified.
                if statement
                    .get("qualifiers")
                    .is_some_and(|v| !v.as_object().is_some_and(|q| q.is_empty()))
                {
                    return None;
                }
                let snak = statement.get("mainsnak")?;
                if snak.get("snaktype")?.as_str()? != "value"
                    || snak.get("property")?.as_str()? != pid
                {
                    return None;
                }
                if snak.get("datatype")?.as_str()? != property.datatype {
                    return None;
                }
                let raw = snak.get("datavalue")?.get("value")?;
                let value = match property.datatype.as_str() {
                    "wikibase-item" => {
                        let id = raw.get("id")?.as_str()?;
                        if !external_id(id, b'Q') || raw.get("entity-type")?.as_str()? != "item" {
                            return None;
                        }
                        let entity = item_entity(id, id);
                        let entity_id = entity.id().ok()?;
                        entities.entry(id.into()).or_insert(entity);
                        RoleValue::Entity { entity_id }
                    }
                    "string" => {
                        let value = raw.as_str()?;
                        if value.len() > 8192 {
                            return None;
                        }
                        RoleValue::Literal {
                            literal: Literal::String(value.into()),
                        }
                    }
                    _ => return None,
                };
                let guid = statement.get("id")?.as_str()?;
                if !guid.starts_with(&format!("{qid}$")) || guid.len() > 256 {
                    return None;
                }
                Some(StructuredAssertion {
                    anchor: format!("{qid}/{pid}/{guid}"),
                    relation: RelationInput {
                        relation_type: format!("wikidata:{pid}@1"),
                        roles: BTreeMap::from([
                            (
                                "subject".into(),
                                vec![RoleValue::Entity {
                                    entity_id: subject_id.clone(),
                                }],
                            ),
                            ("value".into(), vec![value]),
                        ]),
                        qualifiers: BTreeMap::new(),
                        polarity: Polarity::Positive,
                        modality: Modality::Asserted,
                        condition: crate::relation::condition::Condition::True,
                        valid_time: None,
                    },
                    evidence: Vec::new(),
                    epistemic_kind: EpistemicKind::Extracted,
                })
            })();
            match mapped {
                Some(assertion) => assertions.push(assertion),
                None => proposals.push((
                    "unsupported_or_unqualified_wikidata_statement".into(),
                    statement.clone(),
                )),
            }
        }
    }
    let data = ExtractedData {
        entities: entities.into_values().collect(),
        assertions,
    };
    data.validate_bounds()?;
    Ok((data, proposals))
}
fn label<'a>(item: &'a Value, language: &str) -> Option<&'a str> {
    item.get("labels")?.get(language)?.get("value")?.as_str()
}
fn item_entity(qid: &str, name: &str) -> EntityInput {
    EntityInput {
        namespace: "wikidata".into(),
        kind: "wikidata_item".into(),
        canonical_key: qid.into(),
        name: name.into(),
        aliases: Vec::new(),
    }
}
