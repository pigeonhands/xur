use std::sync::Arc;

use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::option;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::regex", |module| {
        module.raw_function("new", regex_new);

        module
            .ty::<Regex>("Regex")
            .raw_function("is_match", regex_is_match)
            .raw_function("find", regex_find)
            .raw_function("find_all", regex_find_all)
            .raw_function("captures", regex_captures)
            .raw_function("replace", regex_replace)
            .raw_function("replace_all", regex_replace_all)
            .raw_function("split", regex_split);
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

fn with_re<R>(value: &Value, f: impl FnOnce(&regex::Regex) -> R) -> RunResult<R> {
    match value {
        Value::Native(obj) => obj
            .with(|r: &Regex| f(&r.0))
            .ok_or_else(|| RuntimeError::new("expected a Regex")),
        _ => Err(RuntimeError::new("expected a Regex")),
    }
}

fn regex_new(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(pattern)] = args else {
        return Err(RuntimeError::new("`new` takes a pattern string"));
    };
    let re = regex::Regex::new(pattern)
        .map_err(|e| RuntimeError::new(format!("invalid regex: {e}")))?;
    into_value(cx, Regex(Arc::new(re)))
}

fn regex_is_match(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`is_match` takes a string"));
    };
    let matched = with_re(this, |re| re.is_match(text))?;
    Ok(Value::Bool(matched))
}

fn regex_find(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`find` takes a string"));
    };
    let found = with_re(this, |re| re.find(text).map(|m| m.as_str().to_string()))?;
    Ok(option(cx, found.map(|s| Value::str(&s))))
}

fn regex_find_all(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`find_all` takes a string"));
    };
    let matches = with_re(this, |re| {
        re.find_iter(text)
            .map(|m| Value::str(m.as_str()))
            .collect::<Vec<_>>()
    })?;
    Ok(Value::list(matches))
}

fn regex_captures(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`captures` takes a string"));
    };
    let caps = with_re(this, |re| {
        re.captures(text).map(|caps| {
            caps.iter()
                .map(|m| m.map(|m| Value::str(m.as_str())).unwrap_or(Value::Unit))
                .collect::<Vec<_>>()
        })
    })?;
    Ok(option(cx, caps.map(Value::list)))
}

fn regex_replace(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text), Value::Str(replacement)] = args else {
        return Err(RuntimeError::new("`replace` takes (text, replacement)"));
    };
    let result = with_re(this, |re| re.replace(text, &**replacement).into_owned())?;
    Ok(Value::str(&result))
}

fn regex_replace_all(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text), Value::Str(replacement)] = args else {
        return Err(RuntimeError::new("`replace_all` takes (text, replacement)"));
    };
    let result = with_re(this, |re| re.replace_all(text, &**replacement).into_owned())?;
    Ok(Value::str(&result))
}

fn regex_split(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(text)] = args else {
        return Err(RuntimeError::new("`split` takes a string"));
    };
    let parts = with_re(this, |re| {
        re.split(text).map(|s| Value::str(s)).collect::<Vec<_>>()
    })?;
    Ok(Value::list(parts))
}
