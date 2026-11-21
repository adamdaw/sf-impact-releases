//! Small checks shared by the closed wire types; JSON Schema stays a test dependency.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub(crate) fn require(valid: bool, why: &str) -> Result<(), String> {
    if valid { Ok(()) } else { Err(why.into()) }
}
pub(crate) fn nonempty(strings: &[&str]) -> Result<(), String> {
    require(strings.iter().all(|s| !s.is_empty()), "empty name or path")
}
pub(crate) fn candidates(strings: &[String]) -> Result<(), String> {
    require(
        strings.len() >= 2
            && strings.iter().all(|s| !s.is_empty())
            && (1..strings.len()).all(|i| !strings[..i].contains(&strings[i])),
        "candidates must contain at least two distinct names",
    )
}
pub(crate) fn id(id: &str) -> Result<(), String> {
    require(
        matches!(id.len(), 40 | 64) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "invalid git object id",
    )
}
/// A nullable field is still required. Serde's default Option decoding otherwise
/// treats a missing key as null.
pub(crate) fn nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}
/// Optional fields must keep their wire presence: explicit null and empty arrays
/// cannot silently turn into absent fields. Required nullable fields cannot vanish.
pub(crate) fn decode<T: Serialize + DeserializeOwned>(given: &Value) -> Result<T, String> {
    let decoded: T = serde_json::from_value(given.clone()).map_err(|e| e.to_string())?;
    let kept = serde_json::to_value(&decoded).map_err(|e| e.to_string())?;
    if let Some(field) = crate::provenance::unread(given, &kept, "")
        .or_else(|| crate::provenance::unread(&kept, given, ""))
    {
        return Err(format!("`{field}` is missing, unknown or invalid"));
    }
    Ok(decoded)
}

/// An optional field, when supplied, must have a non-null value.
pub(crate) fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}
/// Typed decoding from a raw row must retain the location-free diagnostics of
/// the former Value decoder. Syntax errors are still reported by the outer parser.
pub(crate) fn diagnostic(error: serde_json::Error) -> String {
    let mut message = error.to_string();
    if error.line() != 0 {
        let suffix = format!(" at line {} column {}", error.line(), error.column());
        if message.ends_with(&suffix) {
            message.truncate(message.len() - suffix.len());
        }
    }
    message
}
/// Preserve presence separately from an explicitly nullable value.
pub(crate) fn present_nullable<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

macro_rules! optional {
    ($function:ident, $field:literal) => {
        pub(crate) fn $function<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
        where
            D: serde::Deserializer<'de>,
            T: Deserialize<'de>,
        {
            Option::<T>::deserialize(d)?.map(Some).ok_or_else(|| {
                serde::de::Error::custom(concat!("`", $field, "` is missing, unknown or invalid"))
            })
        }
    };
}
optional!(scope, "scope");
optional!(previous_file, "previous_file");
optional!(start, "start");
optional!(end, "end");
optional!(referenced_type, "referenced_type");
macro_rules! list {
    ($function:ident, $field:literal) => {
        pub(crate) fn $function<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
        where
            D: serde::Deserializer<'de>,
            T: Deserialize<'de>,
        {
            let list = Vec::<T>::deserialize(d)?;
            if list.is_empty() {
                return Err(serde::de::Error::custom(concat!(
                    "`",
                    $field,
                    "` is missing, unknown or invalid"
                )));
            }
            Ok(list)
        }
    };
}
list!(gaps, "gaps");
list!(hunks, "hunks");
list!(candidates_list, "candidates");

/// Only failures need a second scan: find duplicate keys even if the first
/// occurrence had the wrong type, before returning the typed diagnostic.
pub(crate) fn row_error(line: &str, error: serde_json::Error) -> String {
    struct Keys;
    impl<'de> serde::de::Visitor<'de> for Keys {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a row object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut keys = std::collections::HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !keys.insert(key.clone()) {
                    map.next_value::<serde::de::IgnoredAny>()?;
                    while map
                        .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                        .is_some()
                    {}
                    return Ok(Some(key));
                }
                map.next_value::<serde::de::IgnoredAny>()?;
            }
            Ok(None)
        }
    }
    let mut d = serde_json::Deserializer::from_str(line);
    if let Ok(Some(field)) = serde::Deserializer::deserialize_map(&mut d, Keys) {
        return format!("duplicate field `{field}`");
    }
    diagnostic(error)
}

/// JSONL readers can retain raw input for shape selection and diagnostics.
/// Serde's untagged document variants instead pass already-buffered content.
/// Adapt that document-only path to RawValue's newtype request too, without
/// depending on serde_json's private raw-value marker or serde's content types.
pub(crate) fn raw_row<'de, D>(d: D) -> Result<Box<serde_json::value::RawValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Input<D>(D);
    struct Buffered<V> {
        name: &'static str,
        visitor: V,
    }
    impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for Buffered<V> {
        type Value = V::Value;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            self.visitor.expecting(f)
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            self.visitor.visit_map(map)
        }
        fn visit_newtype_struct<D: serde::Deserializer<'de>>(
            self,
            d: D,
        ) -> Result<Self::Value, D::Error> {
            let document_value = Value::deserialize(d)?;
            let text = serde_json::to_string(&document_value).map_err(serde::de::Error::custom)?;
            self.visitor
                .visit_map(serde::de::value::MapDeserializer::new(std::iter::once((
                    self.name, text,
                ))))
        }
    }
    impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for Input<D> {
        type Error = D::Error;
        fn deserialize_any<V: serde::de::Visitor<'de>>(
            self,
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            self.0.deserialize_any(visitor)
        }
        fn deserialize_newtype_struct<V: serde::de::Visitor<'de>>(
            self,
            name: &'static str,
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            self.0
                .deserialize_newtype_struct(name, Buffered { name, visitor })
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf option unit unit_struct seq tuple tuple_struct map
            struct enum identifier ignored_any
        }
    }
    Box::<serde_json::value::RawValue>::deserialize(Input(d))
}
