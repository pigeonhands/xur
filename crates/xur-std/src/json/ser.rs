//! `serde::Serialize` for xur values: a `Value` tree is emitted through
//! any serde `Serializer`, giving `std::json::stringify` (and future
//! formats) for free.

use serde::ser::{self, SerializeMap, SerializeSeq};

use xur_runtime::{Value, lock};

use super::JsonValue;

impl ser::Serialize for JsonValue {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_value(&self.0, serializer)
    }
}

fn serialize_value<S: ser::Serializer>(value: &Value, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Value::Unit => serializer.serialize_unit(),
        Value::Bool(b) => serializer.serialize_bool(*b),
        Value::Int(i) => serializer.serialize_i64(*i),
        Value::Float(f) => serializer.serialize_f64(*f),
        Value::Str(s) => serializer.serialize_str(s),
        Value::Tuple(items) => {
            let mut seq = serializer.serialize_seq(Some(items.len()))?;
            for item in items.iter() {
                seq.serialize_element(&JsonValue(item.clone()))?;
            }
            seq.end()
        }
        Value::List(items) => {
            let items = lock(items);
            let mut seq = serializer.serialize_seq(Some(items.len()))?;
            for item in items.iter() {
                seq.serialize_element(&JsonValue(item.clone()))?;
            }
            seq.end()
        }
        Value::Bytes(bytes) => serializer.serialize_bytes(&lock(bytes)),
        Value::Record(record) => {
            let mut map = serializer.serialize_map(Some(record.fields.len()))?;
            for (name, value) in &record.fields {
                map.serialize_entry(name.as_str(), &JsonValue(value.clone()))?;
            }
            map.end()
        }
        Value::Adt(adt) => {
            if adt.fields.is_empty() {
                serializer.serialize_unit()
            } else if adt.fields.len() == 1 {
                serialize_value(&adt.fields[0], serializer)
            } else {
                let mut seq = serializer.serialize_seq(Some(adt.fields.len()))?;
                for field in &adt.fields {
                    seq.serialize_element(&JsonValue(field.clone()))?;
                }
                seq.end()
            }
        }
        Value::Range(range) => {
            let mut map = serializer.serialize_map(Some(3))?;
            map.serialize_entry("start", &range.start)?;
            map.serialize_entry("end", &range.end)?;
            map.serialize_entry("inclusive", &range.inclusive)?;
            map.end()
        }
        // Non-data values: functions, closures, iterators, modules, native objects.
        _ => Err(ser::Error::custom(format!(
            "cannot serialize {} to JSON",
            value.kind()
        ))),
    }
}
