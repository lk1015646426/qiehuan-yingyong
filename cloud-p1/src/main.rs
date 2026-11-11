//! P1: authorized OpenAI-compatible provider gateway. Never reads WorkBuddy sessions.
use reqwest::{blocking::Client, redirect::Policy};
use serde_json::{json, Value};
use std::{
    env,
    io::Read,
    net::{IpAddr, Ipv4Addr},
    sync::{atomic::{AtomicUsize, Ordering}, Arc},
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::{Host, Url};

const MAX_BODY: usize = 1024 * 1024;
const MAX_INFLIGHT: usize = 8;

struct Config {
    model: String,
    client_key: String,
    upstream_key: String,
    endpoint: String,
    client: Client,
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("validated header")
}

fn reply(req: Request, status: u16, v: Value) {
    let _ = req.respond(Response::from_string(v.to_string())
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("X-Content-Type-Options", "nosniff")));
}

fn error(req: Request, status: u16, code: &str, message: &str) {
    reply(req, status, json!({"error":{"type":code,"message":message}}));
}

fn validate_upstream(raw: &str, test_loopback: bool) -> Result<String, String> {
    let url = Url::parse(raw).map_err(|_| "P1_UPSTREAM_BASE_URL must be a valid URL")?;
    if !url.username().is_empty() || url.password().is_some() ||
       url.query().is_some() || url.fragment().is_some() {
        return Err("Provider URL cannot contain credentials, query or fragment".into());
    }
    if !url.path().trim_end_matches('/').ends_with("/v1") {
        return Err("Provider URL must end with /v1 (e.g. /compatible-mode/v1)".into());
    }
    let is_https_host = url.scheme() == "https"
        && matches!(url.host(), Some(Host::Domain(d)) if d != "localhost")
        && url.port_or_known_default() == Some(443);
    let is_test_loopback = test_loopback && url.scheme() == "http"
        && matches!(url.host(), Some(Host::Ipv4(a)) if a.is_loopback());
    if !is_https_host && !is_test_loopback {
        return Err("Only HTTPS domain providers are permitted (or explicit HTTP loopback test)".into());
    }
    Ok(format!("{}/chat/completions", url.as_str().trim_end_matches('/')))
}

fn validate_input(v: &Value, configured_model: &str) -> Result<bool, &'static str> {
    if v.get("model").and_then(Value::as_str) != Some(configured_model) {
        return Err("Model is not enabled for this gateway");
    }
    if !v.get("messages").and_then(Value::as_array).is_some_and(|m| !m.is_empty()) {
        return Err("messages must be a nonempty array");
    }
    if v.get("stream").is_some() && !v["stream"].is_boolean() {
        return Err("stream must be a boolean");
    }
    Ok(v.get("stream").and_then(Value::as_bool).unwrap_or(false))
}

fn handle(mut req: Request, config: &Config) {
    let path = req.url().split('?').next().unwrap_or("").to_owned();
    if req.method() == &Method::Get && path == "/healthz" {
        reply(req, 200, json!({"status":"ok","backend":"authorized-provider","model_ready":true}));
        return;
    }
    let expected = format!("Bearer {}", config.client_key);
    let authorized = req.headers().iter().any(|h| {
        h.field.equiv("Authorization") && h.value.as_str() == expected
    });
    if !authorized {
        error(req, 401, "invalid_api_key", "Missing or invalid client API key");
        return;
    }
    if req.method() == &Method::Get && path == "/v1/models" {
        reply(req, 200, json!({"object":"list","data":[{
            "id":config.model,"object":"model","owned_by":"authorized-provider"
        }]}));
        return;
    }
    if req.method() != &Method::Post || path != "/v1/chat/completions" {
        error(req, 404, "not_found", "Only Chat Completions is supported in P1");
        return;
    }
    let mut body = Vec::new();
    if req.as_reader().take((MAX_BODY+1) as u64).read_to_end(&mut body).is_err() {
        error(req, 400, "invalid_request", "Cannot read request body"); return;
    }
    if body.len() > MAX_BODY {
        error(req, 413, "request_too_large", "Maximum request body: 1 MiB"); return;
    }
    let json: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {error(req,400,"invalid_json","Request must be JSON"); return;}
    };
    let stream = match validate_input(&json, &config.model) {
        Ok(stream) => stream,
        Err(message) => {error(req,400,"invalid_request",message); return;}
    };
    let accept = if stream {"text/event-stream"} else {"application/json"};
    let upstream = config.client.post(&config.endpoint)
        .bearer_auth(&config.upstream_key)
        .header("Accept", accept)
        .json(&json)
        .send();
    let response = match upstream {
        Ok(resp) => resp,
        Err(_) => {error(req,502,"upstream_network_error","Authorized provider request failed"); return;}
    };
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        // Deliberately never reflect raw provider error bodies or authorization headers.
        if status == 429 {
            error(req,429,"upstream_rate_limited","Authorized provider rate limit reached");
        } else if status == 401 || status == 403 {
            error(req,502,"upstream_auth_error","Provider rejected server-side credentials");
        } else {
            error(req,502,"upstream_error","Provider rejected the request");
        }
        return;
    }
    let content_type = if stream {"text/event-stream; charset=utf-8"} else {"application/json"};
    let _ = req.respond(Response::new(StatusCode(200),vec![
        header("Content-Type",content_type),
        header("Cache-Control","no-cache"),
        header("X-Content-Type-Options","nosniff")
    ], response, None, None));
}

fn config_from_env() -> Result<Config, String> {
    let client_key = env::var("P1_CLIENT_API_KEY").unwrap_or_default();
    if client_key.len() < 24 {
        return Err("Set P1_CLIENT_API_KEY to a unique 24+ character client key".into());
    }
    let upstream_key = env::var("P1_UPSTREAM_API_KEY").unwrap_or_default();
    if upstream_key.len() < 8 {
        return Err("Set P1_UPSTREAM_API_KEY to an authorized provider key".into());
    }
    let model = env::var("P1_MODEL").unwrap_or_default();
    if model.is_empty() || model.len() > 120 || model.chars().any(|c| c.is_control()) {
        return Err("Set P1_MODEL to a single authorized model identifier".into());
    }
    let raw_base = env::var("P1_UPSTREAM_BASE_URL").unwrap_or_default();
    let allow_local = env::var("P1_TEST_ALLOW_HTTP_LOOPBACK").ok().as_deref() == Some("1");
    let endpoint = validate_upstream(&raw_base, allow_local)?;
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| "Cannot create upstream HTTPS client")?;
    Ok(Config {model, client_key, upstream_key, endpoint, client})
}

fn main() {
    let config = match config_from_env() {
        Ok(v) => Arc::new(v),
        Err(e) => {eprintln!("P1 configuration error: {e}"); std::process::exit(2);}
    };
    let bind=env::var("P1_BIND").unwrap_or_else(|_| "127.0.0.1:18791".into());
    let addr: std::net::SocketAddr = match bind.parse() {
        Ok(v)=>v,
        Err(_)=>{eprintln!("Invalid P1_BIND"); std::process::exit(2);}
    };
    if addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) &&
       !addr.ip().is_loopback() {
        eprintln!("P1 refuses non-loopback binding; requires a separate secured TLS gateway");
        std::process::exit(2);
    }
    let server=Server::http(&bind).expect("P1 local loopback bind failed");
    let inflight=Arc::new(AtomicUsize::new(0));
    println!("P1 ready: {} (no desktop WorkBuddy dependency)",bind);
    for request in server.incoming_requests() {
        let count=inflight.fetch_add(1,Ordering::SeqCst);
        if count >= MAX_INFLIGHT {
            inflight.fetch_sub(1,Ordering::SeqCst);
            error(request,429,"gateway_busy","Too many concurrent requests");
            continue;
        }
        let config=Arc::clone(&config);
        let _permit=Permit(Arc::clone(&inflight));
        thread::spawn(move || {let _permit=_permit; handle(request,&config);});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enforces_provider_url_policy() {
        assert!(validate_upstream("https://dashscope.aliyuncs.com/compatible-mode/v1",false).unwrap().ends_with("/v1/chat/completions"));
        assert!(validate_upstream("http://example.com/v1",false).is_err());
        assert!(validate_upstream("https://example.com:444/v1",false).is_err());
        assert!(validate_upstream("http://127.0.0.1:1234/v1",false).is_err());
        assert!(validate_upstream("http://127.0.0.1:1234/v1",true).is_ok());
        assert!(validate_upstream("https://a.com/v1?key=x",false).is_err());
    }
    #[test]
    fn enforces_model_and_messages() {
        assert_eq!(validate_input(&json!({"model":"qwen-plus","messages":[{"role":"user","content":"hi"}],"stream":true}),"qwen-plus"),Ok(true));
        assert!(validate_input(&json!({"model":"other","messages":[{}]}),"qwen-plus").is_err());
        assert!(validate_input(&json!({"model":"qwen-plus","messages":[]}),"qwen-plus").is_err());
    }
}
