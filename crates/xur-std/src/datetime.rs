use chrono::{Datelike, Timelike};
use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::datetime", |module| {
        module
            .raw_function("now", dt_now)
            .raw_function("utc_now", dt_utc_now)
            .raw_function("from_timestamp", dt_from_timestamp)
            .raw_function("parse", dt_parse);

        module
            .ty::<DateTime>("DateTime")
            .method("year", |d: &DateTime| d.0.year() as i64)
            .method("month", |d: &DateTime| d.0.month() as i64)
            .method("day", |d: &DateTime| d.0.day() as i64)
            .method("hour", |d: &DateTime| d.0.hour() as i64)
            .method("minute", |d: &DateTime| d.0.minute() as i64)
            .method("second", |d: &DateTime| d.0.second() as i64)
            .method("timestamp", |d: &DateTime| d.0.timestamp())
            .method("timestamp_millis", |d: &DateTime| d.0.timestamp_millis())
            .method("weekday", |d: &DateTime| d.0.format("%A").to_string())
            .method("iso", |d: &DateTime| d.0.to_rfc3339())
            .raw_function("format", dt_format)
            .raw_function("add_days", dt_add_days)
            .raw_function("add_hours", dt_add_hours)
            .raw_function("add_minutes", dt_add_minutes)
            .raw_function("add_seconds", dt_add_seconds)
            .raw_function("add_weeks", dt_add_weeks)
            .raw_function("add_months", dt_add_months)
            .raw_function("add_years", dt_add_years);
    });
}

#[derive(Clone, Facet)]
#[facet(opaque)]
struct DateTime(chrono::DateTime<chrono::FixedOffset>);

impl std::fmt::Display for DateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_rfc3339())
    }
}

fn dt_now(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    if !args.is_empty() {
        return Err(RuntimeError::new("`now` takes no arguments"));
    }
    let now = chrono::Local::now().fixed_offset();
    into_value(cx, DateTime(now))
}

fn dt_utc_now(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    if !args.is_empty() {
        return Err(RuntimeError::new("`utc_now` takes no arguments"));
    }
    let now = chrono::Utc::now().fixed_offset();
    into_value(cx, DateTime(now))
}

fn dt_from_timestamp(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Int(ts)] = args else {
        return Err(RuntimeError::new("`from_timestamp` takes a unix timestamp"));
    };
    let dt = chrono::DateTime::from_timestamp(*ts, 0)
        .ok_or_else(|| RuntimeError::new("invalid timestamp"))?
        .fixed_offset();
    into_value(cx, DateTime(dt))
}

fn dt_parse(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (text, fmt) = match args {
        [Value::Str(text)] => (&**text, None),
        [Value::Str(text), Value::Str(fmt)] => (&**text, Some(&**fmt)),
        _ => {
            return Err(RuntimeError::new(
                "`parse` takes (text) or (text, format)",
            ));
        }
    };
    let dt = if let Some(fmt) = fmt {
        chrono::NaiveDateTime::parse_from_str(text, fmt)
            .map_err(|e| RuntimeError::new(format!("failed to parse datetime: {e}")))?
            .and_utc()
            .fixed_offset()
    } else {
        chrono::DateTime::parse_from_rfc3339(text)
            .or_else(|_| chrono::DateTime::parse_from_rfc2822(text))
            .map_err(|e| RuntimeError::new(format!("failed to parse datetime: {e}")))?
    };
    let value = into_value(cx, DateTime(dt))?;
    Ok(crate::result(cx, Ok(value)))
}

fn with_dt(value: &Value, f: impl FnOnce(&DateTime) -> DateTime) -> RunResult<DateTime> {
    match value {
        Value::Native(obj) => obj
            .with(f)
            .ok_or_else(|| RuntimeError::new("expected a DateTime")),
        _ => Err(RuntimeError::new("expected a DateTime")),
    }
}

fn dt_add_days(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_days` takes an integer"));
    };
    let dt = with_dt(this, |d| DateTime(d.0 + chrono::TimeDelta::days(*n)))?;
    into_value(cx, dt)
}

fn dt_add_hours(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_hours` takes an integer"));
    };
    let dt = with_dt(this, |d| DateTime(d.0 + chrono::TimeDelta::hours(*n)))?;
    into_value(cx, dt)
}

fn dt_add_minutes(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_minutes` takes an integer"));
    };
    let dt = with_dt(this, |d| DateTime(d.0 + chrono::TimeDelta::minutes(*n)))?;
    into_value(cx, dt)
}

fn dt_add_seconds(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_seconds` takes an integer"));
    };
    let dt = with_dt(this, |d| DateTime(d.0 + chrono::TimeDelta::seconds(*n)))?;
    into_value(cx, dt)
}

fn dt_add_weeks(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_weeks` takes an integer"));
    };
    let dt = with_dt(this, |d| DateTime(d.0 + chrono::TimeDelta::weeks(*n)))?;
    into_value(cx, dt)
}

fn dt_add_months(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_months` takes an integer"));
    };
    let dt = if *n >= 0 {
        with_dt(this, |d| DateTime(d.0 + chrono::Months::new(*n as u32)))?
    } else {
        with_dt(this, |d| DateTime(d.0 - chrono::Months::new((-*n) as u32)))?
    };
    into_value(cx, dt)
}

fn dt_add_years(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_years` takes an integer"));
    };
    let months = *n * 12;
    let dt = if months >= 0 {
        with_dt(this, |d| DateTime(d.0 + chrono::Months::new(months as u32)))?
    } else {
        with_dt(this, |d| DateTime(d.0 - chrono::Months::new((-months) as u32)))?
    };
    into_value(cx, dt)
}

fn dt_format(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(fmt)] = args else {
        return Err(RuntimeError::new("`format` takes a format string"));
    };
    let formatted = match this {
        Value::Native(obj) => obj
            .with(|d: &DateTime| d.0.format(fmt).to_string())
            .ok_or_else(|| RuntimeError::new("expected a DateTime"))?,
        _ => return Err(RuntimeError::new("expected a DateTime")),
    };
    Ok(Value::str(&formatted))
}
