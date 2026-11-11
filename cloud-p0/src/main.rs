//! Headless API contract mock. No WorkBuddy tokens or upstream calls.
use serde_json::{json, Value};
use std::{env, io::{Read, self}, thread, time::Duration};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const MODEL: &str = "workbuddy-pool-p0-mock";
const MAX_BODY: usize = 1024 * 1024;

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}
fn json_reply(req: Request, code: u16, v: Value) {
    let _ = req.respond(Response::from_string(v.to_string())
        .with_status_code(StatusCode(code))
        .with_header(header("Content-Type", "application/json; charset=utf-8")));
}
fn fail(req: Request, code: u16, message: &str) {
    json_reply(req, code, json!({"error":{"message":message,"type":"p0_mock_error"}}));
}
fn validate(v: &Value) -> Result<bool, &'static str> {
    if v.get("model").and_then(Value::as_str) != Some(MODEL) {
        return Err("Unknown model");
    }
    if !v.get("messages").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return Err("messages must be a non-empty array");
    }
    if v.get("stream").is_some() && !v["stream"].is_boolean() {
        return Err("stream must be boolean");
    }
    Ok(v.get("stream").and_then(Value::as_bool).unwrap_or(false))
}
struct Frames { data: Vec<Vec<u8>>, index: usize, offset: usize }
impl Frames {
    fn new(data: Vec<String>) -> Self {
        Self {data:data.into_iter().map(String::into_bytes).collect(), index:0, offset:0}
    }
}
impl Read for Frames {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if b.is_empty() { return Ok(0); }
        loop {
            if self.index >= self.data.len() { return Ok(0); }
            let f = &self.data[self.index];
            if self.offset >= f.len() {
                self.index += 1; self.offset = 0;
                if self.index < self.data.len() { thread::sleep(Duration::from_millis(35)); }
                continue;
            }
            let n = b.len().min(f.len()-self.offset);
            b[..n].copy_from_slice(&f[self.offset..self.offset+n]);
            self.offset += n;
            return Ok(n);
        }
    }
}
fn stream_frames() -> Vec<String> {
    vec![
        format!("data: {}\n\n",json!({"id":"p0","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]})),
        format!("data: {}\n\n",json!({"id":"p0","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"P0_MOCK_OK"},"finish_reason":null}]})),
        format!("data: {}\n\n",json!({"id":"p0","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})),
        "data: [DONE]\n\n".to_owned()
    ]
}
fn serve(mut req: Request, api_key: &str) {
    let path = req.url().split('?').next().unwrap_or("");
    if req.method() == &Method::Get && path == "/healthz" {
        json_reply(req,200,json!({"status":"ok","upstream":"disabled","mode":"mock"}));
        return;
    }
    let expected = format!("Bearer {api_key}");
    if !req.headers().iter().any(|h| h.field.equiv("Authorization") && h.value.as_str() == expected) {
        fail(req,401,"Missing or invalid Bearer key"); return;
    }
    if req.method() == &Method::Get && path == "/v1/models" {
        json_reply(req,200,json!({"object":"list","data":[{"id":MODEL,"object":"model","owned_by":"p0-mock"}]}));
        return;
    }
    if req.method() != &Method::Post || path != "/v1/chat/completions" {
        fail(req,404,"Unknown endpoint. Responses API not implemented."); return;
    }
    let mut bytes=Vec::new();
    if req.as_reader().take((MAX_BODY+1) as u64).read_to_end(&mut bytes).is_err() {
        fail(req,400,"Cannot read body"); return;
    }
    if bytes.len()>MAX_BODY { fail(req,413,"Body too large"); return; }
    let v:Value=match serde_json::from_slice(&bytes) {
        Ok(v)=>v,Err(_)=>{fail(req,400,"Invalid JSON"); return;}
    };
    match validate(&v) {
        Err(msg)=> fail(req,400,msg),
        Ok(true)=>{
            let _=req.respond(Response::new(StatusCode(200),vec![
                header("Content-Type","text/event-stream; charset=utf-8"),
                header("Cache-Control","no-cache"),
                header("X-Content-Type-Options","nosniff")
            ],Frames::new(stream_frames()),None,None));
        }
        Ok(false)=>json_reply(req,200,json!({
            "id":"p0","object":"chat.completion","model":MODEL,
            "choices":[{"index":0,"message":{"role":"assistant","content":"P0_MOCK_OK"},"finish_reason":"stop"}]
        }))
    }
}
fn main() {
    let key=env::var("P0_API_KEY").unwrap_or_default();
    if key.len()<16 { eprintln!("Set P0_API_KEY to a local test key (16+ chars)"); std::process::exit(2); }
    let bind=env::var("P0_BIND").unwrap_or_else(|_|"127.0.0.1:18790".into());
    if !bind.starts_with("127.0.0.1:") && !bind.starts_with("[::1]:") {
        eprintln!("P0 mock refuses non-loopback binding"); std::process::exit(2);
    }
    let server=Server::http(&bind).expect("P0 local bind");
    println!("P0 mock listening at {bind}; no real upstream");
    for req in server.incoming_requests() {
        let key=key.clone();
        thread::spawn(move || serve(req,&key));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn validates_modes() {
        for v in [true,false] {
            assert_eq!(validate(&json!({"model":MODEL,"messages":[{"role":"user","content":"test"}],"stream":v})),Ok(v));
        }
    }
    #[test] fn rejects_bad_inputs() {
        assert!(validate(&json!({"model":"other","messages":[{}]})).is_err());
        assert!(validate(&json!({"model":MODEL,"messages":[]})).is_err());
        assert!(validate(&json!({"model":MODEL,"messages":[{}],"stream":"true"})).is_err());
    }
    #[test] fn stream_is_complete() {
        let s=stream_frames().concat();
        assert!(s.contains("P0_MOCK_OK") && s.ends_with("data: [DONE]\n\n"));
    }
    #[test] fn streaming_reader_preserves_frames() {
        let mut r=Frames::new(vec!["one".into(),"two".into()]);
        let mut text=String::new();
        r.read_to_string(&mut text).unwrap();
        assert_eq!(text,"onetwo");
    }
}
