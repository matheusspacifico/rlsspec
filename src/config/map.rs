use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_saphyr::Spanned;

#[derive(Debug)]
pub struct SpannedMap<V>(Vec<(Spanned<String>, V)>);

impl<V> Default for SpannedMap<V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<V> SpannedMap<V> {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Spanned<String>, V)> {
        self.0.iter()
    }
}

impl<V> IntoIterator for SpannedMap<V> {
    type Item = (Spanned<String>, V);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for SpannedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for MapVisitor<V> {
            type Value = SpannedMap<V>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a mapping")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(SpannedMap(entries))
            }
        }

        deserializer.deserialize_map(MapVisitor(PhantomData))
    }
}
