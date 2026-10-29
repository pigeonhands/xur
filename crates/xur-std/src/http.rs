use std::io::Read as _;
use std::sync::Arc;

use facet::Facet;
use ureq::ResponseExt;
use xur_runtime::convert::into_value;
use xur_runtime::{Context, NativeObject, Registry, RunResult, RuntimeError, Value, lock};

use crate::{error_text, result};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::http", |module| {
        module
            .raw_function("get", http_get)
            .raw_function("post", http_post)
            .raw_function("put", http_put)
            .raw_function("patch", http_patch)
            .raw_function("delete", http_delete)
            .raw_function("head", http_head)
            .raw_function("request", http_request);

        module
            .ty::<RequestBuilder>("RequestBuilder")
            .raw_function("header", rb_header)
            .raw_function("bearer_auth", rb_bearer_auth)
            .raw_function("basic_auth", rb_basic_auth)
            .raw_function("body", rb_body)
            .raw_function("json_body", rb_json_body)
            .raw_function("timeout", rb_timeout)
            .raw_function("query", rb_query)
            .raw_function("send", rb_send);

        module
            .ty::<Response>("Response")
            .method("status", |r: &Response| r.status as i64)
            .method("status_text", |r: &Response| r.status_text.clone())
            .method("url", |r: &Response| r.url.clone())
            .method("text", |r: &Response| {
                String::from_utf8_lossy(&r.body).into_owned()
            })
            .raw_function("bytes", resp_bytes)
            .raw_function("json", resp_json)
            .raw_function("header", resp_header)
            .raw_function("headers", resp_headers);
    });
}

// ── Response ───────────────────────────────────────────────────────

#[derive(Clone, Facet)]
#[facet(opaque)]
struct Response {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Arc<Vec<u8>>,
    url: String,
}

impl std::fmt::Display for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Response {{ {} {} }}", self.status, self.status_text)
    }
}

fn read_response(resp: ureq::http::Response<ureq::Body>) -> RunResult<Response> {
    let status = resp.status().as_u16();
    let status_text = resp
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let url = resp.get_uri().to_string();

    let mut headers = Vec::new();
    for (name, value) in resp.headers() {
        headers.push((name.to_string(), value.to_str().unwrap_or("").to_string()));
    }

    let mut body_bytes = Vec::new();
    resp.into_body()
        .as_reader()
        .read_to_end(&mut body_bytes)
        .map_err(|e| RuntimeError::new(format!("failed to read response body: {e}")))?;

    Ok(Response {
        status,
        status_text,
        headers,
        body: Arc::new(body_bytes),
        url,
    })
}

fn with_response<R>(value: &Value, f: impl FnOnce(&Response) -> R) -> RunResult<R> {
    match value {
        Value::Native(obj) => obj
            .with(f)
            .ok_or_else(|| RuntimeError::new("expected an HTTP Response")),
        _ => Err(RuntimeError::new("expected an HTTP Response")),
    }
}

fn resp_bytes(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this] = args else {
        return Err(RuntimeError::new("`bytes` takes no arguments"));
    };
    let bytes = with_response(this, |r| r.body.as_ref().clone())?;
    Ok(Value::bytes(bytes))
}

fn resp_json(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this] = args else {
        return Err(RuntimeError::new("`json` takes no arguments"));
    };
    let text = with_response(this, |r| String::from_utf8_lossy(&r.body).into_owned())?;
    match crate::json::parse_str(cx, &text) {
        Ok(value) => Ok(result(cx, Ok(value))),
        Err(e) => Ok(result(cx, Err(e.message))),
    }
}

fn resp_header(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(name)] = args else {
        return Err(RuntimeError::new("`header` takes a header name string"));
    };
    let value = with_response(this, |r| {
        let lower = name.to_lowercase();
        r.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.clone())
    })?;
    Ok(crate::option(cx, value.map(|v| Value::str(&v))))
}

fn resp_headers(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this] = args else {
        return Err(RuntimeError::new("`headers` takes no arguments"));
    };
    let headers = with_response(this, |r| r.headers.clone())?;
    let fields: Vec<(xur_span::Symbol, Value)> = headers
        .into_iter()
        .map(|(k, v)| (xur_span::Symbol::intern(&k), Value::str(&v)))
        .collect();
    let _ = cx;
    Ok(Value::Record(xur_runtime::Record::new(fields).into()))
}

// ── RequestBuilder ─────────────────────────────────────────────────

#[derive(Clone, Facet)]
#[facet(opaque)]
struct RequestBuilder {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    content_type: Option<String>,
    timeout_secs: Option<u64>,
    query_params: Vec<(String, String)>,
}

impl std::fmt::Display for RequestBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RequestBuilder {{ {} {} }}", self.method, self.url)
    }
}

fn with_rb(value: &Value, f: impl FnOnce(&RequestBuilder) -> RequestBuilder) -> RunResult<Value> {
    match value {
        Value::Native(obj) => {
            let new = obj
                .with(f)
                .ok_or_else(|| RuntimeError::new("expected a RequestBuilder"))?;
            let new_obj = NativeObject::new(obj.ty, Box::new(new));
            Ok(Value::Native(new_obj.into()))
        }
        _ => Err(RuntimeError::new("expected a RequestBuilder")),
    }
}

fn rb_header(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(name), Value::Str(value)] = args else {
        return Err(RuntimeError::new("`header` takes (name, value) strings"));
    };
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.headers.push((name.to_string(), value.to_string()));
        rb
    })
}

fn rb_bearer_auth(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(token)] = args else {
        return Err(RuntimeError::new("`bearer_auth` takes a token string"));
    };
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.headers
            .push(("authorization".into(), format!("Bearer {token}")));
        rb
    })
}

fn rb_basic_auth(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(user), Value::Str(pass)] = args else {
        return Err(RuntimeError::new(
            "`basic_auth` takes (username, password) strings",
        ));
    };
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        let encoded = base64_encode(format!("{user}:{pass}").as_bytes());
        rb.headers
            .push(("authorization".into(), format!("Basic {encoded}")));
        rb
    })
}

fn rb_body(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (this, body) = match args {
        [this, Value::Str(s)] => (this, s.as_bytes().to_vec()),
        [this, Value::Bytes(b)] => (this, lock(b).clone()),
        _ => return Err(RuntimeError::new("`body` takes a string or bytes value")),
    };
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.body = Some(body.clone());
        rb
    })
}

fn rb_json_body(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, value] = args else {
        return Err(RuntimeError::new("`json_body` takes a value to serialize"));
    };
    let json_str = crate::json::stringify_value(cx, value)?;
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.body = Some(json_str.clone().into_bytes());
        rb.content_type = Some("application/json".into());
        rb
    })
}

fn rb_timeout(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Int(secs)] = args else {
        return Err(RuntimeError::new("`timeout` takes a number of seconds"));
    };
    let secs = *secs as u64;
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.timeout_secs = Some(secs);
        rb
    })
}

fn rb_query(_cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this, Value::Str(key), Value::Str(value)] = args else {
        return Err(RuntimeError::new("`query` takes (key, value) strings"));
    };
    with_rb(this, |rb| {
        let mut rb = rb.clone();
        rb.query_params.push((key.to_string(), value.to_string()));
        rb
    })
}

fn rb_send(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [this] = args else {
        return Err(RuntimeError::new("`send` takes no arguments"));
    };
    let rb = match this {
        Value::Native(obj) => obj
            .with(|rb: &RequestBuilder| rb.clone())
            .ok_or_else(|| RuntimeError::new("expected a RequestBuilder"))?,
        _ => return Err(RuntimeError::new("expected a RequestBuilder")),
    };
    match execute_request(&rb) {
        Ok(resp) => {
            let value = into_value(cx, resp)?;
            Ok(result(cx, Ok(value)))
        }
        Err(e) => Ok(result(cx, Err(e.message))),
    }
}

fn execute_request(rb: &RequestBuilder) -> RunResult<Response> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(
            rb.timeout_secs.unwrap_or(30),
        )))
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let has_body = rb.body.is_some()
        || matches!(rb.method.as_str(), "POST" | "PUT" | "PATCH");

    let resp = if has_body {
        let mut req = match rb.method.as_str() {
            "PUT" => agent.put(&rb.url),
            "PATCH" => agent.patch(&rb.url),
            _ => agent.post(&rb.url),
        };
        for (k, v) in &rb.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        for (k, v) in &rb.query_params {
            req = req.query(k.as_str(), v.as_str());
        }
        if let Some(ct) = &rb.content_type {
            req = req.header("content-type", ct.as_str());
        }
        let body_data = rb.body.clone().unwrap_or_default();
        req.send(body_data)
    } else {
        let mut req = match rb.method.as_str() {
            "DELETE" => agent.delete(&rb.url),
            "HEAD" => agent.head(&rb.url),
            "OPTIONS" => agent.options(&rb.url),
            _ => agent.get(&rb.url),
        };
        for (k, v) in &rb.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        for (k, v) in &rb.query_params {
            req = req.query(k.as_str(), v.as_str());
        }
        req.call()
    };

    match resp {
        Ok(resp) => read_response(resp),
        Err(ureq::Error::StatusCode(code)) => {
            Err(RuntimeError::new(format!("HTTP error: status {code}")))
        }
        Err(e) => Err(RuntimeError::new(error_text(e))),
    }
}

// ── Shorthand functions ────────────────────────────────────────────

fn http_get(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(url)] = args else {
        return Err(RuntimeError::new("`get` takes a URL string"));
    };
    simple_request(cx, "GET", url, None)
}

fn http_post(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (url, body) = match args {
        [Value::Str(url)] => (url, None),
        [Value::Str(url), Value::Str(body)] => (url, Some(body.as_bytes().to_vec())),
        [Value::Str(url), Value::Bytes(body)] => (url, Some(lock(body).clone())),
        _ => return Err(RuntimeError::new("`post` takes (url) or (url, body)")),
    };
    simple_request(cx, "POST", url, body)
}

fn http_put(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (url, body) = match args {
        [Value::Str(url)] => (url, None),
        [Value::Str(url), Value::Str(body)] => (url, Some(body.as_bytes().to_vec())),
        [Value::Str(url), Value::Bytes(body)] => (url, Some(lock(body).clone())),
        _ => return Err(RuntimeError::new("`put` takes (url) or (url, body)")),
    };
    simple_request(cx, "PUT", url, body)
}

fn http_patch(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let (url, body) = match args {
        [Value::Str(url)] => (url, None),
        [Value::Str(url), Value::Str(body)] => (url, Some(body.as_bytes().to_vec())),
        [Value::Str(url), Value::Bytes(body)] => (url, Some(lock(body).clone())),
        _ => return Err(RuntimeError::new("`patch` takes (url) or (url, body)")),
    };
    simple_request(cx, "PATCH", url, body)
}

fn http_delete(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(url)] = args else {
        return Err(RuntimeError::new("`delete` takes a URL string"));
    };
    simple_request(cx, "DELETE", url, None)
}

fn http_head(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(url)] = args else {
        return Err(RuntimeError::new("`head` takes a URL string"));
    };
    simple_request(cx, "HEAD", url, None)
}

fn http_request(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [Value::Str(method), Value::Str(url)] = args else {
        return Err(RuntimeError::new("`request` takes (method, url) strings"));
    };
    let rb = RequestBuilder {
        method: method.to_uppercase(),
        url: url.to_string(),
        headers: Vec::new(),
        body: None,
        content_type: None,
        timeout_secs: None,
        query_params: Vec::new(),
    };
    into_value(cx, rb)
}

fn simple_request(
    cx: &mut dyn Context,
    method: &str,
    url: &str,
    body: Option<Vec<u8>>,
) -> RunResult<Value> {
    let rb = RequestBuilder {
        method: method.into(),
        url: url.into(),
        headers: Vec::new(),
        body,
        content_type: None,
        timeout_secs: None,
        query_params: Vec::new(),
    };
    match execute_request(&rb) {
        Ok(resp) => {
            let value = into_value(cx, resp)?;
            Ok(result(cx, Ok(value)))
        }
        Err(e) => Ok(result(cx, Err(e.message))),
    }
}

// ── Base64 (minimal, for Basic auth) ───────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(n >> 18 & 63) as usize] as char);
        out.push(CHARS[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(n >> 6 & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
