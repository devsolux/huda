//! Three-valued, bounded domain conditions. Never used to manufacture caller permissions.
use super::model::{identifier, stable_id, Literal};
use crate::error::{require, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Truth {
    True,
    False,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    #[default]
    True,
    False,
    All {
        args: Vec<Condition>,
    },
    Any {
        args: Vec<Condition>,
    },
    Not {
        arg: Box<Condition>,
    },
    Eq {
        key: String,
        value: Literal,
    },
    Neq {
        key: String,
        value: Literal,
    },
    In {
        key: String,
        values: Vec<Literal>,
    },
    Gt {
        key: String,
        value: Literal,
    },
    Gte {
        key: String,
        value: Literal,
    },
    Lt {
        key: String,
        value: Literal,
    },
    Lte {
        key: String,
        value: Literal,
    },
    Exists {
        key: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "state",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Fact {
    Present(Literal),
    AuthoritativelyAbsent,
    Unknown,
}
pub type Facts = BTreeMap<String, Fact>;
impl Condition {
    pub fn normalize(&mut self) -> Result<()> {
        self.validate(0, &mut 0)?;
        match self {
            Self::All { args } | Self::Any { args } => {
                for arg in args.iter_mut() {
                    arg.normalize()?;
                }
                let mut keyed = args
                    .drain(..)
                    .map(|a| Ok((stable_id("condition", &a)?, a)))
                    .collect::<Result<Vec<_>>>()?;
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                keyed.dedup_by(|a, b| a.0 == b.0);
                *args = keyed.into_iter().map(|(_, a)| a).collect();
            }
            Self::Not { arg } => arg.normalize()?,
            Self::In { values, .. } => {
                values.sort();
                values.dedup();
            }
            _ => {}
        }
        Ok(())
    }
    fn validate(&self, depth: usize, nodes: &mut usize) -> Result<()> {
        *nodes += 1;
        require(
            depth <= 8 && *nodes <= 128,
            "LIMIT_EXCEEDED",
            "Condition AST exceeds limits",
        )?;
        match self {
            Self::All { args } | Self::Any { args } => {
                require(
                    !args.is_empty() && args.len() <= 16,
                    "INVALID_INPUT",
                    "Condition list must contain 1..16 items",
                )?;
                for arg in args {
                    arg.validate(depth + 1, nodes)?;
                }
            }
            Self::Not { arg } => arg.validate(depth + 1, nodes)?,
            Self::In { key, values } => {
                identifier(key)?;
                require(
                    !values.is_empty() && values.len() <= 32,
                    "LIMIT_EXCEEDED",
                    "Membership list exceeds limits",
                )?;
                for v in values {
                    v.validate()?;
                }
            }
            Self::Eq { key, value }
            | Self::Neq { key, value }
            | Self::Gt { key, value }
            | Self::Gte { key, value }
            | Self::Lt { key, value }
            | Self::Lte { key, value } => {
                identifier(key)?;
                value.validate()?;
            }
            Self::Exists { key } => identifier(key)?,
            Self::True | Self::False => {}
        }
        Ok(())
    }
    pub fn evaluate(&self, facts: &Facts) -> Truth {
        use Truth::*;
        fn not(t: Truth) -> Truth {
            match t {
                True => False,
                False => True,
                Unknown => Unknown,
            }
        }
        match self {
            Self::True => True,
            Self::False => False,
            Self::Not { arg } => not(arg.evaluate(facts)),
            Self::All { args } => {
                let mut unknown = false;
                for a in args {
                    match a.evaluate(facts) {
                        False => return False,
                        Unknown => unknown = true,
                        _ => {}
                    }
                }
                if unknown {
                    Unknown
                } else {
                    True
                }
            }
            Self::Any { args } => {
                let mut unknown = false;
                for a in args {
                    match a.evaluate(facts) {
                        True => return True,
                        Unknown => unknown = true,
                        _ => {}
                    }
                }
                if unknown {
                    Unknown
                } else {
                    False
                }
            }
            Self::Exists { key } => match facts.get(key) {
                Some(Fact::Present(_)) => True,
                Some(Fact::AuthoritativelyAbsent) => False,
                _ => Unknown,
            },
            Self::Eq { key, value } | Self::Neq { key, value } => match facts.get(key) {
                Some(Fact::Present(actual)) if actual.type_name() == value.type_name() => {
                    let t = if actual == value { True } else { False };
                    if matches!(self, Self::Neq { .. }) {
                        not(t)
                    } else {
                        t
                    }
                }
                _ => Unknown,
            },
            Self::In { key, values } => match facts.get(key) {
                Some(Fact::Present(actual)) => {
                    if values.contains(actual) {
                        True
                    } else {
                        False
                    }
                }
                _ => Unknown,
            },
            Self::Gt { key, value }
            | Self::Gte { key, value }
            | Self::Lt { key, value }
            | Self::Lte { key, value } => {
                let Some(Fact::Present(actual)) = facts.get(key) else {
                    return Unknown;
                };
                // Never compare numeric strings lexicographically. Unsupported typed order stays UNKNOWN.
                let ordering = match (actual, value) {
                    (Literal::Integer(a), Literal::Integer(b))
                    | (Literal::Timestamp(a), Literal::Timestamp(b)) => a.cmp(b),
                    _ => return Unknown,
                };
                let ok = match self {
                    Self::Gt { .. } => ordering.is_gt(),
                    Self::Gte { .. } => !ordering.is_lt(),
                    Self::Lt { .. } => ordering.is_lt(),
                    _ => !ordering.is_gt(),
                };
                if ok {
                    True
                } else {
                    False
                }
            }
        }
    }
}
