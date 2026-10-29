use std::time;

use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::time", |module| {
        module
            .raw_function("now", instant_now)
            .raw_function("sleep", time_sleep)
            .raw_function("from_secs", dur_from_secs)
            .raw_function("from_millis", dur_from_millis);

        module
            .ty::<Instant>("Instant")
            .raw_function("elapsed", instant_elapsed);

        module
            .ty::<Duration>("Duration")
            .method("secs", |d: &Duration| d.0.as_secs() as i64)
            .method("millis", |d: &Duration| d.0.as_millis() as i64)
            .method("micros", |d: &Duration| d.0.as_micros() as i64)
            .method("nanos", |d: &Duration| d.0.as_nanos() as i64)
            .method("as_secs_f64", |d: &Duration| d.0.as_secs_f64());
    });
}

// ── Instant ────────────────────────────────────────────────────────

#[derive(Clone, Facet)]
#[facet(opaque)]
struct Instant(time::Instant);

impl std::fmt::Display for Instant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Instant {{ {:?} }}", self.0)
    }
}

fn instant_now(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    if !args.is_empty() {
        return Err(RuntimeError::new("`now` takes no arguments"));
    }
    into_value(cx, Instant(time::Instant::now()))
}

fn instant_elapsed(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this] = args else {
        return Err(RuntimeError::new("`elapsed` takes no arguments"));
    };
    let elapsed = match this {
        Value::Native(obj) => obj
            .with(|i: &Instant| i.0.elapsed())
            .ok_or_else(|| RuntimeError::new("expected an Instant"))?,
        _ => return Err(RuntimeError::new("expected an Instant")),
    };
    into_value(cx, Duration(elapsed))
}

// ── Duration ───────────────────────────────────────────────────────

#[derive(Clone, Facet)]
#[facet(opaque)]
struct Duration(time::Duration);

impl std::fmt::Display for Duration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = self.0;
        if d.as_secs() > 0 {
            write!(f, "{:.3}s", d.as_secs_f64())
        } else if d.as_millis() > 0 {
            write!(f, "{}ms", d.as_millis())
        } else if d.as_micros() > 0 {
            write!(f, "{}µs", d.as_micros())
        } else {
            write!(f, "{}ns", d.as_nanos())
        }
    }
}

fn dur_from_secs(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let secs = match args {
        [Value::Int(n)] => time::Duration::from_secs(*n as u64),
        [Value::Float(f)] => time::Duration::from_secs_f64(*f),
        _ => return Err(RuntimeError::new("`from_secs` takes a number")),
    };
    into_value(cx, Duration(secs))
}

fn dur_from_millis(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Int(n)] = args else {
        return Err(RuntimeError::new("`from_millis` takes an integer"));
    };
    into_value(cx, Duration(time::Duration::from_millis(*n as u64)))
}

fn time_sleep(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Int(ms)] => {
            std::thread::sleep(time::Duration::from_millis(*ms as u64));
        }
        [Value::Native(obj)] => {
            let dur = obj
                .with(|d: &Duration| d.0)
                .ok_or_else(|| RuntimeError::new("`sleep` takes milliseconds or a Duration"))?;
            std::thread::sleep(dur);
        }
        _ => return Err(RuntimeError::new("`sleep` takes milliseconds or a Duration")),
    }
    Ok(Value::Unit)
}
