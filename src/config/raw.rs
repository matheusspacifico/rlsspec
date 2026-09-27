use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_saphyr::Spanned;

use super::map::SpannedMap;
use super::{Expectation, Unspecified};

pub type Value = Spanned<Option<String>>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    pub version: Spanned<u32>,
    pub database: RawDatabase,
    #[serde(default)]
    pub safety: RawSafety,
    #[serde(default)]
    pub setup: Vec<Spanned<String>>,
    #[serde(default)]
    pub vars: SpannedMap<Spanned<String>>,
    pub identities: Spanned<SpannedMap<RawIdentity>>,
    #[serde(default)]
    pub unspecified: Unspecified,
    #[serde(default)]
    pub defaults: SpannedMap<SpannedMap<RawOps>>,
    #[serde(default)]
    pub expect: SpannedMap<SpannedMap<RawOps>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDatabase {
    pub url: Spanned<String>,
    pub schemas: Option<Spanned<Vec<Spanned<String>>>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSafety {
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIdentity {
    pub role: Spanned<String>,
    #[serde(default)]
    pub gucs: SpannedMap<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOps {
    pub select: Option<Spanned<RawSelect>>,
    pub insert: Option<Spanned<RawWrites<RawInsert>>>,
    pub update: Option<Spanned<RawWrites<RawUpdate>>>,
    pub delete: Option<Spanned<RawWrites<RawDelete>>>,
}

#[derive(Debug)]
pub enum RawSelect {
    Deny,
    All,
    Rows { rows: Spanned<String>, subset: bool },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowsForm {
    rows: Spanned<String>,
    #[serde(default)]
    subset: bool,
}

impl<'de> Deserialize<'de> for RawSelect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SelectVisitor;

        impl<'de> Visitor<'de> for SelectVisitor {
            type Value = RawSelect;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`deny`, `all` or a mapping with `rows`")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<RawSelect, E> {
                match v {
                    "deny" => Ok(RawSelect::Deny),
                    "all" => Ok(RawSelect::All),
                    _ => Err(E::invalid_value(de::Unexpected::Str(v), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<RawSelect, A::Error> {
                let form = RowsForm::deserialize(MapAccessDeserializer::new(map))?;
                Ok(RawSelect::Rows {
                    rows: form.rows,
                    subset: form.subset,
                })
            }
        }

        deserializer.deserialize_any(SelectVisitor)
    }
}

#[derive(Debug)]
pub enum RawWrites<C> {
    Shorthand(Expectation),
    Cases(Vec<Spanned<C>>),
}

impl<'de, C: Deserialize<'de>> Deserialize<'de> for RawWrites<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WritesVisitor<C>(PhantomData<C>);

        impl<'de, C: Deserialize<'de>> Visitor<'de> for WritesVisitor<C> {
            type Value = RawWrites<C>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`allow`, `deny` or a list of cases")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<RawWrites<C>, E> {
                match v {
                    "allow" => Ok(RawWrites::Shorthand(Expectation::Allow)),
                    "deny" => Ok(RawWrites::Shorthand(Expectation::Deny)),
                    _ => Err(E::invalid_value(de::Unexpected::Str(v), &self)),
                }
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<RawWrites<C>, A::Error> {
                Vec::deserialize(SeqAccessDeserializer::new(seq)).map(RawWrites::Cases)
            }
        }

        deserializer.deserialize_any(WritesVisitor(PhantomData))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInsert {
    pub values: Spanned<SpannedMap<Value>>,
    pub expect: Expectation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawUpdate {
    #[serde(rename = "where")]
    pub predicate: Spanned<String>,
    pub set: Option<Spanned<SpannedMap<Value>>>,
    pub expect: Expectation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDelete {
    #[serde(rename = "where")]
    pub predicate: Spanned<String>,
    pub expect: Expectation,
}
