use chrono::{Datelike, Timelike};
use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::datetime", |module| {
        module
            .function("now", || DateTime(chrono::Local::now().fixed_offset()))
            .function("utc_now", || DateTime(chrono::Utc::now().fixed_offset()))
            .named_function("from_timestamp", &["secs"], |ts: i64| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| DateTime(dt.fixed_offset()))
                    .ok_or_else(|| "invalid timestamp".to_string())
            })
            .raw_function_returns::<DateTime>("parse", dt_parse);

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
            .method("format", |d: &DateTime, fmt: String| {
                d.0.format(&fmt).to_string()
            })
            .method("add_days", |d: &DateTime, n: i64| {
                DateTime(d.0 + chrono::TimeDelta::days(n))
            })
            .method("add_hours", |d: &DateTime, n: i64| {
                DateTime(d.0 + chrono::TimeDelta::hours(n))
            })
            .method("add_minutes", |d: &DateTime, n: i64| {
                DateTime(d.0 + chrono::TimeDelta::minutes(n))
            })
            .method("add_seconds", |d: &DateTime, n: i64| {
                DateTime(d.0 + chrono::TimeDelta::seconds(n))
            })
            .method("add_weeks", |d: &DateTime, n: i64| {
                DateTime(d.0 + chrono::TimeDelta::weeks(n))
            })
            .raw_function_returns::<DateTime>("add_months", dt_add_months)
            .raw_function_returns::<DateTime>("add_years", dt_add_years);
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

fn dt_add_months(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_months` takes an integer"));
    };
    let dt = with_dt(this, |d| {
        if *n >= 0 {
            DateTime(d.0 + chrono::Months::new(*n as u32))
        } else {
            DateTime(d.0 - chrono::Months::new((-*n) as u32))
        }
    })?;
    into_value(cx, dt)
}

fn dt_add_years(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(n)] = args else {
        return Err(RuntimeError::new("`add_years` takes an integer"));
    };
    let months = *n * 12;
    let dt = with_dt(this, |d| {
        if months >= 0 {
            DateTime(d.0 + chrono::Months::new(months as u32))
        } else {
            DateTime(d.0 - chrono::Months::new((-months) as u32))
        }
    })?;
    into_value(cx, dt)
}

fn with_dt(value: &Value, f: impl FnOnce(&DateTime) -> DateTime) -> RunResult<DateTime> {
    match value {
        Value::Native(obj) => obj
            .with(f)
            .ok_or_else(|| RuntimeError::new("expected a DateTime")),
        _ => Err(RuntimeError::new("expected a DateTime")),
    }
}
