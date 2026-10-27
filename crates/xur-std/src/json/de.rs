//! `serde::Deserialize` for xur values: a JSON (or any format) document
//! becomes a `Value` tree without knowing the target type.

use std::fmt;
use std::sync::Arc;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use xur_runtime::{Record, Value};
use xur_span::Symbol;

use super::JsonValue;

impl<'de> de::Deserialize<'de> for JsonValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = JsonValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<JsonValue, E> {
        Ok(JsonValue(Value::Bool(v)))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<JsonValue, E> {
        Ok(JsonValue(Value::Int(v)))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<JsonValue, E> {
        match i64::try_from(v) {
            Ok(i) => Ok(JsonValue(Value::Int(i))),
            Err(_) => Ok(JsonValue(Value::Float(v as f64))),
        }
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<JsonValue, E> {
        Ok(JsonValue(Value::Float(v)))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<JsonValue, E> {
        Ok(JsonValue(Value::str(v)))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<JsonValue, E> {
        Ok(JsonValue(Value::str(&v)))
    }

    fn visit_none<E: de::Error>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::null())
    }

    fn visit_unit<E: de::Error>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::null())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(JsonValue(v)) = seq.next_element()? {
            items.push(v);
        }
        Ok(JsonValue(Value::list(items)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
        let mut fields = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, JsonValue(value))) = map.next_entry::<String, JsonValue>()? {
            fields.push((Symbol::intern(&key), value));
        }
        Ok(JsonValue(Value::Record(Arc::new(Record::new(fields)))))
    }
}

/// Allows `JsonValue` to be used as a map value / sequence element
/// without an extra wrapper.
impl<'de> DeserializeSeed<'de> for JsonVisitor {
    type Value = JsonValue;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        de::Deserialize::deserialize(deserializer)
    }
}
