use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer, StrDeserializer};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_saphyr::Spanned;

use super::map::SpannedMap;
use super::{Expectation, Unspecified};

pub type Value = Spanned<Option<String>>;

/// Top-level keys with this prefix are accepted and ignored: a home for shared YAML anchors.
const EXTENSION_PREFIX: &str = "x-";

// `remote = "Self"` makes the derive an inherent function, so the trait impl below can wrap it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, remote = "Self")]
pub struct RawConfig {
    pub version: Spanned<u32>,
    pub preset: Option<Spanned<String>>,
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
    #[serde(default)]
    pub lint: RawLint,
}

impl<'de> Deserialize<'de> for RawConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ConfigVisitor;

        impl<'de> Visitor<'de> for ConfigVisitor {
            type Value = RawConfig;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a mapping")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<RawConfig, A::Error> {
                RawConfig::deserialize(MapAccessDeserializer::new(SkipExtensions(map)))
            }
        }

        deserializer.deserialize_map(ConfigVisitor)
    }
}

/// The top-level mapping without its `x-*` entries, whose values are skipped unread.
struct SkipExtensions<A>(A);

impl<'de, A: MapAccess<'de>> MapAccess<'de> for SkipExtensions<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        while let Some(key) = self.0.next_key::<String>()? {
            if key.starts_with(EXTENSION_PREFIX) {
                self.0.next_value::<IgnoredAny>()?;
                continue;
            }
            return seed
                .deserialize(StrDeserializer::<A::Error>::new(&key))
                .map(Some);
        }
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(seed)
    }
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
    pub lock_timeout: Option<Spanned<String>>,
    pub statement_timeout: Option<Spanned<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLint {
    #[serde(default)]
    pub ignore: Vec<Spanned<RawIgnore>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIgnore {
    pub rule: Spanned<String>,
    pub table: Option<Spanned<String>>,
    pub identity: Option<Spanned<String>>,
    pub function: Option<Spanned<String>>,
    pub view: Option<Spanned<String>>,
    pub reason: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIdentity {
    pub role: Spanned<String>,
    #[serde(default)]
    pub gucs: SpannedMap<Spanned<String>>,
    pub claims: Option<Spanned<serde_json::Value>>,
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
    Todo,
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
                f.write_str("`deny`, `all`, `todo` or a mapping with `rows`")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<RawSelect, E> {
                match v {
                    "todo" => Ok(RawSelect::Todo),
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
    Todo,
    Shorthand(Expectation),
    Cases(Vec<Spanned<C>>),
}

impl<'de, C: Deserialize<'de>> Deserialize<'de> for RawWrites<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WritesVisitor<C>(PhantomData<C>);

        impl<'de, C: Deserialize<'de>> Visitor<'de> for WritesVisitor<C> {
            type Value = RawWrites<C>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`allow`, `deny`, `todo` or a list of cases")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<RawWrites<C>, E> {
                match v {
                    "todo" => Ok(RawWrites::Todo),
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
