use std::sync::Arc;

use facet::Facet;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::option;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::regex", |module| {
        module.named_function("new", &["pattern"], |pattern: String| {
            regex::Regex::new(&pattern)
                .map(|re| Regex(Arc::new(re)))
                .map_err(|e| format!("invalid regex: {e}"))
        });

        module
            .ty::<Regex>("Regex")
            .method("is_match", |re: &Regex, text: String| re.0.is_match(&text))
            .method("find", |re: &Regex, text: String| {
                re.0.find(&text).map(|m| m.as_str().to_string())
            })
            .method("find_all", |re: &Regex, text: String| {
                re.0.find_iter(&text)
                    .map(|m| m.as_str().to_string())
                    .collect::<Vec<_>>()
            })
            .raw_function("captures", regex_captures)
            .method("replace", |re: &Regex, text: String, replacement: String| {
                re.0.replace(&text, replacement.as_str()).into_owned()
            })
            .method("replace_all", |re: &Regex, text: String, replacement: String| {
                re.0.replace_all(&text, replacement.as_str()).into_owned()
            })
            .method("split", |re: &Regex, text: String| {
                re.0.split(&text).map(|s| s.to_string()).collect::<Vec<_>>()
            });
    });
}

#[derive(Clone, Facet)]
#[facet(opaque)]
struct Regex(Arc<regex::Regex>);

impl std::fmt::Display for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{}/", self.0.as_str())
    }
}

fn regex_captures(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`captures` takes a string"));
    };
    let Value::Native(obj) = this else {
        return Err(RuntimeError::new("expected a Regex"));
    };
    let caps = obj
        .with(|r: &Regex| {
            r.0.captures(text).map(|caps| {
                caps.iter()
                    .map(|m| m.map(|m| Value::str(m.as_str())).unwrap_or(Value::Unit))
                    .collect::<Vec<_>>()
            })
        })
        .ok_or_else(|| RuntimeError::new("expected a Regex"))?;
    Ok(option(cx, caps.map(Value::list)))
}
