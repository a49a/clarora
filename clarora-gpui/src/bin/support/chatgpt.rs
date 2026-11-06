//! Real OAuth callback, signed identity, protected storage and Responses HTTP fixtures.
//! Fake OS vault stays in this process; no user's browser, account or keychain is accessed.
// Responses failure scenarios, recorded before extending the HTTP fixtures:
// - Valid SSE is rejected because Content-Type uses mixed case, a charset parameter,
//   text/plain, or no Content-Type; a completed final event lacks its trailing blank line.
// - A complete JSON Response is rejected, or accepted only after a second paid request.
// - HTTP 200 hides an error object/string, detail response, HTML interstitial, unfinished
//   Response, empty/malformed body, or completed Response without assistant output_text.
// - A completed-looking JSON body also carries error/detail and is falsely accepted.
// - Diagnostics lose HTTP status/request ID/body shape or echo secrets from the body.
// - Any response path drops question/reference/history, or retries an unsuccessful call.
// Additional boundaries recorded before adding their HTTP cases:
// - Valid SSE starts with id:/retry: and is incorrectly fixed into the JSON state.
// - A body closes before its advertised Content-Length and loses transport diagnostics.
// - Secrets occur directly in error strings or error.code, bypassing message redaction.
// - Cancellation from the final JSON progress callback still returns a successful answer.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use clarora_gpui::{ai, backup, chatgpt, data::Library, process};
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
struct Vault(Arc<Mutex<BTreeMap<String, Vec<u8>>>>);
struct Entry(Vault, String);
impl CredentialApi for Entry {
    fn set_secret(&self, v: &[u8]) -> keyring::Result<()> {
        self.0.0.lock().unwrap().insert(self.1.clone(), v.into());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.0
            .0
            .lock()
            .unwrap()
            .get(&self.1)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.0.0.lock().unwrap().remove(&self.1);
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl CredentialBuilderApi for Vault {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(Entry(self.clone(), format!("{service}/{user}"))))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[derive(Default)]
struct ServerState {
    nonce: String,
    challenge: String,
    client: String,
    subject: String,
    case: String,
    refreshes: usize,
    requests: Vec<Value>,
    exchanges: usize,
    callback_pages: Vec<String>,
}
struct Fixture {
    base: String,
    state: Arc<Mutex<ServerState>>,
    stop: process::Cancel,
    worker: Option<thread::JoinHandle<Result<()>>>,
}
fn request(socket: &mut TcpStream) -> Result<(String, BTreeMap<String, String>, Vec<u8>)> {
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = vec![];
    let mut block = [0; 8192];
    let split = loop {
        let n = socket.read(&mut block)?;
        ensure!(n > 0, "request closed");
        bytes.extend_from_slice(&block[..n]);
        ensure!(bytes.len() < 2 * 1024 * 1024, "request too large");
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let text = String::from_utf8_lossy(&bytes[..split]);
    let path = text
        .lines()
        .next()
        .context("request line")?
        .split_whitespace()
        .nth(1)
        .context("path")?
        .to_string();
    let headers = text
        .lines()
        .skip(1)
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    ensure!(length < 2 * 1024 * 1024, "body too large");
    while bytes.len() < split + length {
        let n = socket.read(&mut block)?;
        ensure!(n > 0, "body closed");
        bytes.extend_from_slice(&block[..n]);
    }
    Ok((path, headers, bytes[split..split + length].into()))
}
fn response(s: &mut TcpStream, status: u16, kind: &str, body: &str) -> Result<()> {
    response_length(s, status, kind, body, body.len())
}
fn response_length(
    s: &mut TcpStream,
    status: u16,
    kind: &str,
    body: &str,
    length: usize,
) -> Result<()> {
    let content_type = if kind.is_empty() {
        String::new()
    } else {
        format!("Content-Type: {kind}\r\n")
    };
    write!(
        s,
        "HTTP/1.1 {status} Fixture\r\n{content_type}x-request-id: fixture-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        length
    )?;
    Ok(())
}
fn event(kind: &str, mut value: Value) -> String {
    value["type"] = json!(kind);
    format!("data: {value}\n\n")
}
const RESPONSE_SECRET: &str = "private-access-token fixture-code fixture@example.invalid";
fn json_response(status: &str) -> Value {
    json!({"object":"response","status":status,"output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"你好 🌱"}]}]})
}
fn response_variant(case: &str, stream: String) -> (&'static str, String) {
    match case {
        "sse-charset" => ("text/event-stream; charset=utf-8", stream),
        "sse-mixed-case" => ("Text/Event-Stream; Charset=UTF-8", stream),
        "sse-plain" => ("text/plain", stream),
        "sse-no-content-type" => ("", stream),
        "sse-eof-completed" => ("text/event-stream", stream.trim_end().into()),
        "sse-id-prefix" => ("text/event-stream", format!("id: fixture-event\n{stream}")),
        "sse-retry-prefix" => ("text/event-stream", format!("retry: 1000\n{stream}")),
        "json-completed" | "json-cancel-completed" => {
            ("application/json", json_response("completed").to_string())
        }
        "json-error-object" => (
            "application/json",
            json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","message":RESPONSE_SECRET}}).to_string(),
        ),
        "json-error-string" => (
            "application/json",
            json!({"error":"unauthorized_client","error_description":RESPONSE_SECRET}).to_string(),
        ),
        "json-error-secret-string" => (
            "application/json",
            json!({"error":RESPONSE_SECRET}).to_string(),
        ),
        "json-error-secret-code" => (
            "application/json",
            json!({"error":{"code":RESPONSE_SECRET}}).to_string(),
        ),
        "json-detail" => ("application/json", json!({"detail":RESPONSE_SECRET}).to_string()),
        "html-200" => (
            "text/html",
            format!("<!doctype html><title>Just a moment...</title>{RESPONSE_SECRET}"),
        ),
        "json-incomplete" => ("application/json", json_response("incomplete").to_string()),
        "json-failed" => ("application/json", json_response("failed").to_string()),
        "json-in-progress" => ("application/json", json_response("in_progress").to_string()),
        "json-completed-error" => {
            let mut value = json_response("completed");
            value["error"] = json!({"code":"subscription_sharing_usage_limit_exceeded","message":RESPONSE_SECRET});
            ("application/json", value.to_string())
        }
        "json-completed-detail" => {
            let mut value = json_response("completed");
            value["detail"] = json!(RESPONSE_SECRET);
            ("application/json", value.to_string())
        }
        "json-no-answer" => (
            "application/json",
            json!({"object":"response","status":"completed","output":[]}).to_string(),
        ),
        "json-non-assistant" => {
            let mut value = json_response("completed");
            value["output"][0]["role"] = json!("user");
            ("application/json", value.to_string())
        }
        "json-empty-answer" => {
            let mut value = json_response("completed");
            value["output"][0]["content"][0]["text"] = json!(" \n");
            ("application/json", value.to_string())
        }
        "empty-json" => ("application/json", String::new()),
        "invalid-json" => ("application/json", format!("{{{RESPONSE_SECRET}")),
        "invalid-plain" => ("text/plain", RESPONSE_SECRET.into()),
        _ => ("text/event-stream", stream),
    }
}
impl Fixture {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(ServerState {
            subject: "account-a".into(),
            ..Default::default()
        }));
        let shared = state.clone();
        let stop = process::cancel_token();
        let stopping = stop.clone();
        let address = base.clone();
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| anyhow::anyhow!("keygen"))?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .map_err(|_| anyhow::anyhow!("key"))?;
        let public = key.public_key().as_ref().to_vec();
        let worker = thread::spawn(move || -> Result<()> {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = listener.accept()?;
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let (path, headers, body) = request(&mut socket)?;
                let mut s = shared.lock().unwrap();
                if path == "/.well-known/openid-configuration" {
                    response(&mut socket,200,"application/json",&json!({"issuer":"https://auth.openai.com","jwks_uri":format!("{address}/jwks"),"revocation_endpoint":format!("{address}/revoke")}).to_string())?;
                } else if path == "/jwks" {
                    response(&mut socket,200,"application/json",&json!({"keys":[{"kty":"EC","crv":"P-256","alg":"ES256","use":"sig","kid":"fixture-key","x":B64.encode(&public[1..33]),"y":B64.encode(&public[33..65])}]}).to_string())?;
                } else if path == "/api/accounts/oauth/token" {
                    s.exchanges += 1;
                    let form = url::form_urlencoded::parse(&body)
                        .into_owned()
                        .collect::<BTreeMap<_, _>>();
                    ensure!(form["client_id"] == s.client, "wrong client ID");
                    ensure!(
                        form["resource"] == "https://api.openai.com/v1",
                        "missing resource"
                    );
                    let failure = match s.case.as_str() {
                        "exchange-region" => Some((
                            403,
                            "application/json",
                            r#"{"error":{"code":"unsupported_country_region_territory","message":"private-access-token fixture-code fixture@example.invalid"}}"#,
                        )),
                        "exchange-client" => Some((
                            403,
                            "application/json",
                            r#"{"error":"unauthorized_client","error_description":"private-access-token fixture-code fixture@example.invalid"}"#,
                        )),
                        "exchange-detail" => Some((
                            403,
                            "application/json",
                            r#"{"detail":"private-access-token fixture-code fixture@example.invalid"}"#,
                        )),
                        "exchange-html" => Some((
                            403,
                            "text/html",
                            "<!doctype html><title>Just a moment...</title>private-access-token fixture-code fixture@example.invalid",
                        )),
                        "exchange-invalid-json" => Some((
                            403,
                            "application/json",
                            "private-access-token fixture-code fixture@example.invalid",
                        )),
                        "exchange-invalid-grant" => {
                            Some((400, "application/json", r#"{"error":"invalid_grant"}"#))
                        }
                        "exchange-temporary" => Some((
                            503,
                            "application/json",
                            r#"{"error":"temporarily_unavailable"}"#,
                        )),
                        "exchange-rate" => Some((
                            429,
                            "application/json",
                            r#"{"error":"rate_limit_exceeded"}"#,
                        )),
                        _ => None,
                    };
                    if let Some((status, kind, body)) = failure {
                        response(&mut socket, status, kind, body)?;
                        continue;
                    }
                    if form["grant_type"] == "refresh_token" {
                        if s.case == "refresh-temporary" {
                            response(
                                &mut socket,
                                503,
                                "application/json",
                                "{\"error\":\"temporarily_unavailable\"}",
                            )?;
                            continue;
                        }
                        if s.case == "refresh-expired" {
                            response(
                                &mut socket,
                                400,
                                "application/json",
                                "{\"error\":\"invalid_grant\"}",
                            )?;
                            continue;
                        }
                        s.refreshes += 1;
                        ensure!(
                            form["refresh_token"] == format!("fake-refresh-{}", s.refreshes - 1),
                            "rotation reused stale token"
                        );
                    } else {
                        ensure!(form["code"] == "fixture-code", "wrong code");
                        ensure!(
                            form["redirect_uri"].starts_with("http://127.0.0.1:"),
                            "wrong redirect"
                        );
                        ensure!(form["code_verifier"].len() >= 43, "bad PKCE");
                        ensure!(
                            B64.encode(Sha256::digest(form["code_verifier"].as_bytes()))
                                == s.challenge,
                            "PKCE verifier does not match challenge"
                        );
                    }
                    let stamp = chrono::Utc::now().timestamp();
                    let payload = json!({"iss":if s.case=="issuer" {"https://evil.invalid"}else{"https://auth.openai.com"},"aud":if s.case=="audience" {"wrong"}else{&s.client},"sub":s.subject,"email":"fixture@example.invalid","exp":stamp+3600,"nonce":if s.case=="nonce" {"wrong"}else{&s.nonce}});
                    let data = format!(
                        "{}.{}",
                        B64.encode(json!({"alg":"ES256","kid":"fixture-key"}).to_string()),
                        B64.encode(payload.to_string())
                    );
                    let sig = key
                        .sign(&rng, data.as_bytes())
                        .map_err(|_| anyhow::anyhow!("sign"))?;
                    let mut fixed = sig.as_ref().to_vec();
                    if s.case == "signature" {
                        fixed[0] ^= 1;
                    }
                    let token = json!({"access_token":format!("fake-access-{}",s.refreshes),"refresh_token":format!("fake-refresh-{}",s.refreshes),"id_token":format!("{data}.{}",B64.encode(fixed)),"token_type":"Bearer","expires_in":3600,"scope":if s.case=="no-scope" {"openid email"}else{"openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"}});
                    response(&mut socket, 200, "application/json", &token.to_string())?;
                } else if path == "/v1/models" {
                    ensure!(
                        headers
                            .get("authorization")
                            .is_some_and(|v| v.starts_with("Bearer fake-access-")),
                        "models missing token"
                    );
                    response(&mut socket,200,"application/json",&json!({"models":[{"slug":"fixture-model","display_name":"Fixture model","visibility":"list"},{"slug":"hidden","visibility":"hide"}]}).to_string())?;
                } else if path == "/v1/responses" {
                    let payload: Value = serde_json::from_slice(&body)?;
                    s.requests.push(payload.clone());
                    ensure!(
                        headers
                            .get("authorization")
                            .is_some_and(|v| v.starts_with("Bearer fake-access-")),
                        "chat missing token"
                    );
                    ensure!(
                        payload["stream"] == true && payload["store"] == false,
                        "wrong Responses mode"
                    );
                    ensure!(
                        payload.get("messages").is_none()
                            && payload.get("temperature").is_none()
                            && payload.get("previous_response_id").is_none(),
                        "unsupported fields"
                    );
                    ensure!(
                        payload["input"]
                            .as_array()
                            .is_some_and(|v| v.iter().all(|m| m["role"] != "system")),
                        "system message in input"
                    );
                    let mut body = event("response.output_text.delta", json!({"delta":"你好 🌱"}));
                    match s.case.as_str() {
                        "failed" => {
                            body += &event(
                                "response.failed",
                                json!({"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}),
                            )
                        }
                        "incomplete" => {
                            body += &event(
                                "response.incomplete",
                                json!({"response":{"status":"incomplete"}}),
                            )
                        }
                        "truncated" | "transport-truncated" => {}
                        _ => {
                            body += &event(
                                "response.completed",
                                json!({"response":{"status":"completed"}}),
                            )
                        }
                    }
                    let (kind, body) = response_variant(&s.case, body);
                    if s.case == "transport-truncated" {
                        let _ = response_length(&mut socket, 200, kind, &body, body.len() + 32);
                    } else {
                        let _ = response(&mut socket, 200, kind, &body);
                    }
                } else if path == "/revoke" {
                    let form = url::form_urlencoded::parse(&body)
                        .into_owned()
                        .collect::<BTreeMap<_, _>>();
                    ensure!(
                        form["client_id"] == s.client && form["token_type_hint"] == "refresh_token",
                        "bad revoke"
                    );
                    response(&mut socket, 200, "application/json", "")?;
                } else {
                    response(&mut socket, 404, "application/json", "{}")?;
                }
            }
            Ok(())
        });
        Ok(Self {
            base,
            state,
            stop,
            worker: Some(worker),
        })
    }
    fn login(
        &self,
        client: &chatgpt::Client,
        profile: Option<&str>,
        cancel: &process::Cancel,
    ) -> Result<chatgpt::Snapshot> {
        let (page_tx, page_rx) = std::sync::mpsc::channel();
        let result = client.sign_in(profile, cancel, |address| {
            let url = url::Url::parse(address)?;
            let params = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
            ensure!(
                params["code_challenge_method"] == "S256" && params["code_challenge"].len() == 43,
                "missing challenge"
            );
            let mut s = self.state.lock().unwrap();
            s.nonce = params["nonce"].clone();
            s.challenge = params["code_challenge"].clone();
            if profile.is_none() {
                ensure!(
                    params["client_id"] == "dynamic_agent_client",
                    "registration entrypoint"
                );
                s.client = format!("oaiapp_{}", clarora_gpui::data::id());
                ensure!(params["agent_name_hint"] == "Clarora", "app name");
            } else {
                ensure!(
                    params["client_id"] == s.client && !params.contains_key("agent_name_hint"),
                    "returning registration changed"
                );
            }
            let mut callback = url::Url::parse(&params["redirect_uri"])?;
            callback.query_pairs_mut().append_pair(
                "state",
                if s.case == "state" {
                    "wrong-state"
                } else {
                    &params["state"]
                },
            );
            if s.case == "denied" {
                callback
                    .query_pairs_mut()
                    .append_pair("error", "access_denied");
            } else {
                callback
                    .query_pairs_mut()
                    .append_pair("code", "fixture-code");
            }
            if s.case != "missing-client" {
                callback.query_pairs_mut().append_pair(
                    "client_id",
                    if s.case == "client-change" {
                        "wrong-client"
                    } else {
                        &s.client
                    },
                );
            }
            if s.case == "duplicate-state" {
                callback
                    .query_pairs_mut()
                    .append_pair("state", &params["state"]);
            }
            let invalid = matches!(s.case.as_str(), "state" | "duplicate-state");
            let cancellation = cancel.clone();
            let callback = callback.to_string();
            let page_tx = page_tx.clone();
            thread::spawn(move || {
                if let Ok(response) = reqwest::blocking::Client::builder()
                    .no_proxy()
                    .build()
                    .and_then(|c| c.get(callback).send())
                {
                    let _ = page_tx.send(response.text().unwrap_or_default());
                }
                if invalid {
                    cancellation.store(true, Ordering::SeqCst);
                }
            });
            Ok(())
        });
        if let Ok(page) = page_rx.recv_timeout(Duration::from_secs(1)) {
            self.state.lock().unwrap().callback_pages.push(page);
        }
        result
    }
    fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(w) = self.worker.take() {
            w.join().map_err(|_| anyhow::anyhow!("fixture panic"))??;
        }
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn response_protocol(
    lib: &Library,
    server: &Fixture,
    cancel: &process::Cancel,
    cases: &mut Vec<Value>,
) -> Result<()> {
    let mut evidence = vec![];
    let mut failures = vec![];
    for (case, success, shape) in [
        ("sse-charset", true, "sse"),
        ("sse-mixed-case", true, "sse"),
        ("sse-plain", true, "sse"),
        ("sse-no-content-type", true, "sse"),
        ("sse-eof-completed", true, "sse"),
        ("sse-id-prefix", true, "sse"),
        ("sse-retry-prefix", true, "sse"),
        ("transport-truncated", false, "transport"),
        ("json-completed", true, "response"),
        ("json-cancel-completed", false, "cancelled"),
        ("json-error-object", false, "error_object"),
        ("json-error-string", false, "error_string"),
        ("json-error-secret-string", false, "error_string"),
        ("json-error-secret-code", false, "error_object"),
        ("json-detail", false, "detail"),
        ("html-200", false, "html"),
        ("json-incomplete", false, "response"),
        ("json-failed", false, "response"),
        ("json-in-progress", false, "response"),
        ("json-completed-error", false, "error_object"),
        ("json-completed-detail", false, "detail"),
        ("json-no-answer", false, "response"),
        ("json-non-assistant", false, "response"),
        ("json-empty-answer", false, "response"),
        ("empty-json", false, "empty"),
        ("invalid-json", false, "non_json"),
        ("invalid-plain", false, "non_json"),
    ] {
        server.state.lock().unwrap().case = case.into();
        let before = server.state.lock().unwrap().requests.len();
        let mut chunks = vec![];
        let cancel_on_progress = case == "json-cancel-completed";
        let request_cancel = if cancel_on_progress {
            process::cancel_token()
        } else {
            cancel.clone()
        };
        let mut cancelled_by_progress = false;
        let result = ai::chat_stream(
            lib,
            "解释 why did",
            "Selected words only",
            &[
                json!({"role":"user","content":"previous question"}),
                json!({"role":"assistant","content":"previous answer"}),
            ],
            &request_cancel,
            |reply| {
                chunks.push(reply.answer.clone());
                if cancel_on_progress && reply.answer == "你好 🌱" {
                    cancelled_by_progress = true;
                    request_cancel.store(true, Ordering::SeqCst);
                }
            },
        );
        let requests = server.state.lock().unwrap().requests[before..].to_vec();
        let request = requests.first().cloned().unwrap_or(Value::Null);
        let mut reasons = vec![];
        if requests.len() != 1 {
            reasons.push("request was missing or automatically retried");
        }
        if request["model"] != "fixture-model"
            || request["stream"] != true
            || request["store"] != false
            || !request["instructions"]
                .as_str()
                .is_some_and(|s| s.contains("<material>\nSelected words only\n</material>"))
            || request["input"]
                != json!([
                    {"role":"user","content":"previous question"},
                    {"role":"assistant","content":"previous answer"},
                    {"role":"user","content":"解释 why did"},
                ])
        {
            reasons.push("model, question, reference, history or Responses fields changed");
        }
        let mut diagnosis = String::new();
        let mut answer = String::new();
        match result {
            Ok(reply) => {
                answer = reply.answer;
                if !success {
                    reasons.push("unsuccessful or invalid response was accepted");
                } else if answer != "你好 🌱" || !chunks.last().is_some_and(|s| s == "你好 🌱")
                {
                    reasons.push("valid response or final progress lost its answer");
                }
            }
            Err(error) => {
                diagnosis = format!("{error:#}");
                if success {
                    reasons.push("valid response was rejected");
                } else if cancel_on_progress {
                    if !cancelled_by_progress || !diagnosis.contains("已取消") {
                        reasons.push("final JSON progress cancellation was not acknowledged");
                    }
                } else {
                    let kind = if case == "html-200" {
                        "text/html"
                    } else if case == "invalid-plain" {
                        "text/plain"
                    } else if case == "transport-truncated" {
                        "text/event-stream"
                    } else {
                        "application/json"
                    };
                    if !diagnosis.contains("HTTP 200")
                        || !diagnosis.contains("fixture-request")
                        || (case != "transport-truncated"
                            && !diagnosis.contains(&format!("response={shape}")))
                        || !diagnosis.contains(&format!("content-type={kind}"))
                    {
                        reasons.push(
                            "safe HTTP, request ID, response shape or MIME diagnosis missing",
                        );
                    }
                }
            }
        }
        let mut secret_leaked = false;
        for secret in [
            "private-access-token",
            "fixture-code",
            "fixture@example.invalid",
        ] {
            if diagnosis.contains(secret) {
                secret_leaked = true;
                diagnosis = diagnosis.replace(secret, "[redacted]");
            }
        }
        if secret_leaked {
            reasons.push("response body secrets leaked into diagnostics");
        }
        let passed = reasons.is_empty();
        if !passed {
            failures.push(case);
        }
        let item = json!({"case":case,"passed":passed,"expected_success":success,"expected_shape":shape,"request_count":requests.len(),"request":request,"answer":answer,"progress":chunks,"cancelled_by_progress":cancelled_by_progress,"diagnosis":diagnosis,"secret_leaked":secret_leaked,"failure_reasons":reasons});
        cases.push(item.clone());
        evidence.push(item);
    }
    server.state.lock().unwrap().case.clear();
    let report = lib.root.join("responses-protocol-report.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(
            &json!({"passed":failures.is_empty(),"cases":evidence,"live_openai_verified":false}),
        )?,
    )?;
    ensure!(
        failures.is_empty(),
        "Responses protocol cases failed: {}; evidence: {}",
        failures.join(", "),
        report.display()
    );
    Ok(())
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(lib.root.join("chatgpt-fixture"))?;
    let mut server = Fixture::new()?;
    keyring::set_default_credential_builder(Box::new(Vault::default()));
    chatgpt::diagnostic::configure(&lib, &server.base)?;
    let client = chatgpt::Client::new(&lib);
    let cancel = process::cancel_token();
    let mut cases = vec![];
    for case in [
        "nonce",
        "signature",
        "issuer",
        "audience",
        "state",
        "duplicate-state",
        "missing-client",
        "denied",
    ] {
        server.state.lock().unwrap().case = case.into();
        let attempt = process::cancel_token();
        ensure!(
            server.login(&client, None, &attempt).is_err(),
            "accepted {case}"
        );
        ensure!(
            !chatgpt::snapshot(&lib)?
                .profiles
                .iter()
                .any(|p| p.connected),
            "invalid identity stored"
        );
        cases.push(json!({"case":case,"passed":true}));
    }
    for (case, status, expected, shape) in [
        ("exchange-region", 403, "地区", "error_object"),
        ("exchange-client", 403, "应用注册", "error_string"),
        ("exchange-detail", 403, "拒绝", "detail"),
        ("exchange-html", 403, "网络防护", "html"),
        ("exchange-invalid-json", 403, "拒绝", "non_json"),
        ("exchange-invalid-grant", 400, "失效", "error_string"),
        ("exchange-temporary", 503, "暂时", "error_string"),
        ("exchange-rate", 429, "频繁", "error_string"),
    ] {
        let failure_lib = Library::open(lib.root.join(case))?;
        chatgpt::diagnostic::configure(&failure_lib, &server.base)?;
        let failure_client = chatgpt::Client::new(&failure_lib);
        server.state.lock().unwrap().case = case.into();
        let exchanges = server.state.lock().unwrap().exchanges;
        let error = server
            .login(&failure_client, None, &cancel)
            .err()
            .context("accepted denied token exchange")?;
        let message = format!("{error:#}");
        fs::write(failure_lib.root.join("failure.txt"), &message)?;
        ensure!(
            message.contains("换取登录凭证")
                && message.contains(&format!("HTTP {status}"))
                && message.contains(expected)
                && message.contains(shape)
                && message.contains("fixture-request"),
            "missing safe exchange diagnosis for {case}: {message}"
        );
        ensure!(
            ![
                "private-access-token",
                "fixture-code",
                "fixture@example.invalid"
            ]
            .iter()
            .any(|v| message.contains(v)),
            "authorization details leaked into error"
        );
        let pending = chatgpt::snapshot(&failure_lib)?;
        ensure!(
            !pending.enabled && pending.profiles.len() == 1 && !pending.profiles[0].connected,
            "failed exchange changed active service or registration"
        );
        let pending_id = pending.profiles[0].id.clone();
        let issued = pending.profiles[0].client_id.clone();
        ensure!(
            pending.login_profile().is_some_and(|p| p.id == pending_id),
            "continue login forgot pending registration"
        );
        ensure!(
            server.state.lock().unwrap().exchanges == exchanges + 1,
            "exchange failure was retried automatically"
        );
        ensure!(
            server
                .state
                .lock()
                .unwrap()
                .callback_pages
                .last()
                .is_some_and(|p| p.contains("不代表登录成功")),
            "browser acknowledgement misleadingly reports login success"
        );
        server.state.lock().unwrap().case.clear();
        let retried = server.login(&failure_client, Some(&pending_id), &cancel)?;
        ensure!(
            retried.profiles.len() == 1
                && retried
                    .current()
                    .is_some_and(|p| p.id == pending_id && p.client_id == issued && p.connected),
            "retry created a different dynamic registration"
        );
        ensure!(
            retried.login_profile().is_some_and(|p| p.id == pending_id),
            "continue login ignored selected account"
        );
        let existing = serde_json::to_value(&retried)?;
        server.state.lock().unwrap().case = case.into();
        ensure!(
            server
                .login(&failure_client, Some(&pending_id), &cancel)
                .is_err()
                && serde_json::to_value(chatgpt::snapshot(&failure_lib)?)? == existing,
            "failed reauthorization changed a working session"
        );
        cases.push(json!({"case":case,"passed":true,"status":status,"shape":shape,"diagnosis":message,"retry_reuses_registration":true,"failed_reauth_preserves_session":true}));
    }
    server.state.lock().unwrap().case.clear();
    let first = server.login(&client, None, &cancel)?;
    let id = first.active.clone();
    ensure!(
        first.enabled
            && first
                .profiles
                .iter()
                .any(|p| p.id == id && p.connected && p.sharing && p.models.len() == 1),
        "login/model discovery failed"
    );
    let host = fs::read(lib.root.join("chatgpt/state.json"))?;
    let again = server.login(&client, Some(&id), &cancel)?;
    ensure!(again.active == id, "reauth changed profile");
    ensure!(
        again.host_id == first.host_id,
        "host ID changed on reauthorization"
    );
    server.state.lock().unwrap().case = "client-change".into();
    ensure!(
        server.login(&client, Some(&id), &cancel).is_err(),
        "accepted changed client ID"
    );
    server.state.lock().unwrap().case.clear();
    server.state.lock().unwrap().subject = "wrong-account".into();
    ensure!(
        server.login(&client, Some(&id), &cancel).is_err(),
        "accepted different verified subject"
    );
    server.state.lock().unwrap().subject = "account-a".into();
    ensure!(
        chatgpt::snapshot(&lib)?.current().unwrap().subject == "account-a",
        "failed reauthorization replaced account"
    );
    cases.push(json!({"case":"returning client host and subject stay bound","passed":true}));
    let mut chunks = vec![];
    let answer = ai::chat_stream(
        &lib,
        "解释 why did",
        "Selected words only",
        &[
            json!({"role":"user","content":"previous question"}),
            json!({"role":"assistant","content":"previous answer"}),
        ],
        &cancel,
        |r| chunks.push(r.answer),
    )?;
    ensure!(
        answer.answer == "你好 🌱" && !chunks.is_empty(),
        "stream failed"
    );
    let requests = server.state.lock().unwrap().requests.clone();
    ensure!(
        requests[0]["instructions"]
            .as_str()
            .is_some_and(|s| s.contains("Selected words only")),
        "reference missing"
    );
    ensure!(
        requests[0]["input"].as_array().unwrap().len() == 3,
        "history missing"
    );
    cases.push(
        json!({"case":"signed OAuth to model discovery to actual assistant stream","passed":true}),
    );
    response_protocol(&lib, &server, &cancel, &mut cases)?;
    for case in ["failed", "incomplete", "truncated"] {
        server.state.lock().unwrap().case = case.into();
        let r = ai::chat_stream(&lib, "q", "", &[], &cancel, |_| {});
        ensure!(r.is_err(), "accepted {case} stream");
        if case == "failed" {
            ensure!(
                r.err().unwrap().to_string().contains("额度"),
                "unclear limit error"
            );
        }
        cases.push(json!({"case":case,"passed":true}));
    }
    server.state.lock().unwrap().case.clear();
    let cancelled = process::cancel_token();
    let during = cancelled.clone();
    let started = Instant::now();
    ensure!(
        ai::chat_stream(&lib, "q", "", &[], &cancelled, move |_| during
            .store(true, Ordering::SeqCst))
        .is_err()
            && started.elapsed() < Duration::from_secs(2),
        "stream cancellation ignored"
    );
    cases.push(json!({"case":"cancelled streamed output is not a complete answer","passed":true}));
    chatgpt::diagnostic::expire(&lib)?;
    client.refresh_models(&cancel)?;
    ensure!(
        server.state.lock().unwrap().refreshes == 1,
        "refresh missing"
    );
    cases.push(json!({"case":"rotating refresh saved across reopen","passed":true}));
    chatgpt::diagnostic::expire(&lib)?;
    thread::scope(|scope| -> Result<()> {
        let a = scope.spawn(|| client.refresh_models(&cancel));
        let b = scope.spawn(|| {
            chatgpt::Client::new(&Library::open(lib.root.clone()).unwrap()).refresh_models(&cancel)
        });
        a.join().unwrap()?;
        b.join().unwrap()?;
        Ok(())
    })?;
    ensure!(
        server.state.lock().unwrap().refreshes == 2,
        "concurrent refresh raced"
    );
    cases.push(
        json!({"case":"serialized refresh across independently opened clients","passed":true}),
    );
    chatgpt::diagnostic::expire(&lib)?;
    server.state.lock().unwrap().case = "refresh-temporary".into();
    ensure!(
        client.refresh_models(&cancel).is_err()
            && chatgpt::snapshot(&lib)?.current().unwrap().connected,
        "temporary refresh error erased tokens"
    );
    server.state.lock().unwrap().case.clear();
    client.refresh_models(&cancel)?;
    cases.push(json!({"case":"temporary refresh failure retains renewable session","passed":true}));
    // Same email, different verified subject and registration: never merge profiles.
    server.state.lock().unwrap().subject = "account-b".into();
    let second = server.login(&client, None, &cancel)?;
    let other = second.active.clone();
    ensure!(
        second.host_id == first.host_id
            && other != id
            && second.current().unwrap().subject == "account-b",
        "account switch mixed identities"
    );
    client.select_profile(&id)?;
    let a = lib.root.join(format!("chatgpt/{id}.session"));
    let b = lib.root.join(format!("chatgpt/{other}.session"));
    let original = fs::read(&a)?;
    fs::copy(&b, &a)?;
    ensure!(
        client.refresh_models(&cancel).is_err(),
        "accepted another profile ciphertext"
    );
    fs::write(&a, original)?;
    server.state.lock().unwrap().subject = "account-a".into();
    server.state.lock().unwrap().client = first.current().unwrap().client_id.clone();
    client.refresh_models(&cancel)?;
    cases.push(json!({"case":"separate registrations and encrypted sessions for matching emails","passed":true}));
    server.state.lock().unwrap().case = "no-scope".into();
    let denied = server.login(&client, None, &cancel)?;
    ensure!(
        denied.enabled && denied.current().is_some_and(|p| p.connected && !p.sharing),
        "missing grant silently selected API billing"
    );
    let before = server.state.lock().unwrap().requests.len();
    ensure!(
        ai::chat_stream(&lib, "q", "", &[], &cancel, |_| {}).is_err()
            && server.state.lock().unwrap().requests.len() == before,
        "ungranted account sent inference"
    );
    client.select_profile(&id)?;
    server.state.lock().unwrap().case.clear();
    server.state.lock().unwrap().client = first.current().unwrap().client_id.clone();
    cases.push(
        json!({"case":"declined plan grant blocks inference without API fallback","passed":true}),
    );
    let zip = lib.root.join("backup.zip");
    backup::export_local(&lib, &zip)?;
    let db = lib.db()?;
    let entries: Vec<String> = db
        .prepare("SELECT value FROM app_settings UNION ALL SELECT value FROM local_credentials")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    ensure!(
        entries
            .iter()
            .all(|s| !s.contains("fake-access-") && !s.contains("fake-refresh-")),
        "tokens leaked into SQLite"
    );
    for entry in fs::read_dir(lib.root.join("chatgpt"))? {
        let p = entry?.path();
        if p.is_file() {
            let bytes = fs::read(p)?;
            let text = String::from_utf8_lossy(&bytes);
            ensure!(
                !text.contains("fake-access-") && !text.contains("fake-refresh-"),
                "plain tokens on disk"
            );
        }
    }
    let mut archive = zip::ZipArchive::new(fs::File::open(&zip)?)?;
    for i in 0..archive.len() {
        let mut e = archive.by_index(i)?;
        let mut bytes = vec![];
        e.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        ensure!(
            !text.contains("fake-access-")
                && !text.contains("fake-refresh-")
                && !e.name()?.contains("chatgpt/"),
            "credentials exported"
        );
    }
    cases.push(
        json!({"case":"encrypted local credentials excluded from SQLite and backup","passed":true}),
    );
    let (signed_out, revoked) = client.sign_out(&cancel)?;
    ensure!(
        revoked
            && !signed_out
                .profiles
                .iter()
                .any(|p| p.id == id && p.connected),
        "logout failed"
    );
    ensure!(
        client.refresh_models(&cancel).is_err(),
        "logout left usable tokens"
    );
    cases.push(
        json!({"case":"revoke then remove tokens and reject further requests","passed":true}),
    );
    let stopped = process::cancel_token();
    stopped.store(true, Ordering::SeqCst);
    let started = Instant::now();
    ensure!(
        client
            .sign_in(None, &stopped, |_| anyhow::bail!(
                "browser called after cancel"
            ))
            .is_err()
            && started.elapsed() < Duration::from_secs(1),
        "cancel ignored"
    );
    cases.push(json!({"case":"cancel before authorization does not open browser","passed":true}));
    client.select_profile(&id)?;
    server.login(&client, Some(&id), &cancel)?;
    chatgpt::diagnostic::expire(&lib)?;
    server.state.lock().unwrap().case = "refresh-expired".into();
    ensure!(
        client.refresh_models(&cancel).is_err()
            && !chatgpt::snapshot(&lib)?.current().unwrap().connected,
        "terminal refresh did not invalidate local session"
    );
    cases.push(
        json!({"case":"confirmed invalid refresh token requires reauthorization","passed":true}),
    );
    server.finish()?;
    fs::write(
        lib.root.join("chatgpt-report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"requests":requests,"initial_public_state":serde_json::from_slice::<Value>(&host)?,"live_openai_verified":false}),
        )?,
    )?;
    results.push(json!({"workflow":"ChatGPT会员 OAuth/验签/Responses/刷新/退出/备份隔离","passed":true,"report":lib.root.join("chatgpt-report.json")}));
    Ok(())
}
