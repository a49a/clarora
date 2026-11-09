use super::*;
use serde_json::json;
use std::sync::atomic::Ordering;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_FRAME: usize = 8 * 1024 * 1024;

fn network_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_connect)
    {
        anyhow::anyhow!(
            "网络连接失败（无法完成与 ChatGPT 的 TLS 连接）。请检查网络、代理或 VPN 后重试。原始错误：{error}"
        )
    } else {
        error
    }
}

pub(super) async fn read_json(mut response: reqwest::Response, cancel: &Cancel) -> Result<Value> {
    let mut bytes = vec![];
    while let Some(chunk) = crate::ai_chat::request(cancel, response.chunk()).await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "ChatGPT 服务响应过长"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).context("ChatGPT 服务返回的内容无效，请稍后重试")
}
fn safe(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_-.:".contains(*c))
        .take(120)
        .collect()
}
fn failure(status: u16, value: &Value, request_id: &str) -> anyhow::Error {
    let code = value["error"]["code"]
        .as_str()
        .or_else(|| value["error"].as_str())
        .or_else(|| value["code"].as_str())
        .unwrap_or("");
    // An arbitrary `error` string may be a whole page/message with private data.
    // Only recognized server error enums can be included in copied diagnostics.
    let code = match code {
        "subscription_sharing_usage_limit_exceeded"
        | "subscription_sharing_user_not_eligible"
        | "subscription_sharing_usage_unavailable"
        | "subscription_sharing_user_unavailable"
        | "subscription_sharing_unsupported_capability"
        | "subscription_sharing_route_not_supported"
        | "subscription_sharing_invalid_user"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context"
        | "unauthorized_client"
        | "invalid_client"
        | "access_denied"
        | "invalid_request"
        | "invalid_request_error"
        | "unsupported_country_region_territory"
        | "rate_limit_exceeded"
        | "server_error" => code,
        _ => "",
    };
    let message = match code {
        "subscription_sharing_usage_limit_exceeded" => {
            "ChatGPT 会员在此应用的使用额度已达上限，请到 ChatGPT 设置查看用量或稍后再试"
        }
        "subscription_sharing_user_not_eligible" => {
            "当前 ChatGPT 账户或工作区暂不支持会员授权使用，请检查会员与工作区权限"
        }
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            "ChatGPT 会员服务暂时不可用，请稍后重试"
        }
        "subscription_sharing_unsupported_capability"
        | "subscription_sharing_route_not_supported" => {
            "ChatGPT 会员暂不支持此请求能力，请更新应用或使用其他已配置服务"
        }
        "subscription_sharing_invalid_user"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => {
            "ChatGPT 登录或会员授权不可用，请在设置中检查账户并重新授权"
        }
        _ => match status {
            401 => "ChatGPT 身份或授权未被接受，请在设置中检查账户并重新登录",
            403 => "ChatGPT 拒绝了此请求，请检查账户、工作区、服务地区及授权资格",
            429 => "ChatGPT 请求过于频繁或额度不足，请查看会员用量后重试",
            500..=599 => "ChatGPT 服务暂时不可用，请稍后重试",
            _ => "ChatGPT 请求未完成，请检查模型或稍后重试",
        },
    };
    let code = safe(code);
    let id = safe(request_id);
    anyhow::anyhow!(
        "{message}（HTTP {status}{}{}）",
        if code.is_empty() {
            String::new()
        } else {
            format!(" · {code}")
        },
        if id.is_empty() {
            String::new()
        } else {
            format!(" · 请求 {id}")
        }
    )
}

struct ResponseInfo {
    status: u16,
    kind: String,
    request: String,
    edge: String,
}
impl ResponseInfo {
    fn new(response: &reqwest::Response) -> Self {
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        Self {
            status: response.status().as_u16(),
            kind: header("content-type"),
            request: header("x-request-id"),
            edge: header("cf-ray"),
        }
    }
    fn error(&self, message: impl std::fmt::Display, shape: &str) -> anyhow::Error {
        // Only public response metadata is copied; never include raw server text.
        let kind = self
            .kind
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || "/-_.;= ".contains(*c))
            .take(120)
            .collect::<String>();
        anyhow::anyhow!(
            "{message}\n诊断：HTTP {}; content-type={kind}; response={shape}; request={}; edge={}",
            self.status,
            safe(&self.request),
            safe(&self.edge)
        )
    }
}
fn response_shape(value: &Value) -> &'static str {
    if value["error"].is_object() {
        "error_object"
    } else if value["error"].is_string() {
        "error_string"
    } else if value.get("detail").is_some_and(|v| !v.is_null()) {
        "detail"
    } else if value["object"] == "response" {
        "response"
    } else {
        "json"
    }
}
fn output_text(value: &Value) -> Result<String> {
    let mut answer = String::new();
    if let Some(output) = value["output"].as_array() {
        for item in output {
            if item["type"] != "message" || item["role"] != "assistant" {
                continue;
            }
            ensure!(
                item.get("status").is_none_or(|v| v == "completed"),
                "ChatGPT 回答未完整生成"
            );
            if let Some(parts) = item["content"].as_array() {
                for part in parts {
                    ensure!(
                        part["type"] != "refusal",
                        "ChatGPT 未能回答此问题，请调整问题后重试"
                    );
                    if part["type"] == "output_text" {
                        append(
                            &mut answer,
                            part["text"].as_str().context("ChatGPT 回答正文格式无效")?,
                        )?;
                    }
                }
            }
        }
    }
    Ok(answer)
}
fn json_reply(value: &Value, info: &ResponseInfo) -> Result<ChatReply> {
    let shape = response_shape(value);
    if value.get("error").is_some_and(|v| !v.is_null()) || shape == "detail" {
        return Err(info.error(failure(info.status, value, &info.request), shape));
    }
    ensure!(
        value["object"] == "response" && value["status"] == "completed",
        "{}",
        info.error("ChatGPT 未返回已完成的回答；问题和引用已保留", shape)
    );
    let answer = output_text(value).map_err(|e| info.error(e, shape))?;
    ensure!(
        !answer.trim().is_empty(),
        "{}",
        info.error("ChatGPT 返回的已完成响应中没有回答正文", shape)
    );
    Ok(ChatReply {
        answer,
        reasoning: String::new(),
    })
}
// A few leading bytes identify SSE without trusting a missing or rewritten MIME header.
fn is_event_stream(bytes: &[u8], declared: bool) -> Option<bool> {
    if bytes.len() < 3 && b"\xef\xbb\xbf".starts_with(bytes) {
        return None;
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let bytes = &bytes[bytes.iter().take_while(|b| b.is_ascii_whitespace()).count()..];
    if bytes.is_empty() {
        return None;
    }
    if bytes.starts_with(b":")
        || [b"data:".as_slice(), b"event:", b"id:", b"retry:"]
            .iter()
            .any(|p| bytes.starts_with(p))
    {
        return Some(true);
    }
    if [b"data:".as_slice(), b"event:", b"id:", b"retry:"]
        .iter()
        .any(|p| p.starts_with(bytes))
    {
        return None;
    }
    // JSON/HTML are still inspected even when a proxy incorrectly labels them SSE.
    Some(declared && !matches!(bytes[0], b'{' | b'[' | b'<'))
}
async fn receive(
    mut response: reqwest::Response,
    cancel: &Cancel,
    mut progress: impl FnMut(ChatReply),
) -> Result<ChatReply> {
    let info = ResponseInfo::new(&response);
    let declared = info
        .kind
        .split(';')
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"));
    let mut pending = vec![];
    let mut streaming = None;
    let mut reply = ChatReply::default();
    let mut completed = false;
    while let Some(chunk) = crate::ai_chat::request(cancel, response.chunk())
        .await
        .map_err(|error| {
            if cancel.load(Ordering::SeqCst) {
                error
            } else {
                info.error(
                    "ChatGPT 连接在读取回答时中断，请重试",
                    if streaming == Some(true) {
                        "sse"
                    } else {
                        "transport"
                    },
                )
            }
        })?
    {
        ensure!(
            pending.len() + chunk.len() <= MAX_FRAME,
            "{}",
            info.error(
                "ChatGPT 返回内容过长",
                if streaming == Some(true) {
                    "sse"
                } else {
                    "non_json"
                }
            )
        );
        pending.extend_from_slice(&chunk);
        if streaming.is_none() {
            streaming = is_event_stream(&pending, declared);
        }
        if !(200..300).contains(&info.status) || streaming == Some(false) {
            continue;
        }
        if streaming.is_none() {
            continue;
        }
        loop {
            let lf = pending
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, 2));
            let crlf = pending
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| (i, 4));
            let Some((i, len)) = lf.into_iter().chain(crlf).min_by_key(|v| v.0) else {
                break;
            };
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            completed = frame(&pending[..i], &mut reply, &info.request)
                .map_err(|e| info.error(e, "sse"))?;
            pending.drain(..i + len);
            progress(reply.clone());
            if completed {
                break;
            }
        }
        if completed {
            break;
        }
    }
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    if !(200..300).contains(&info.status) || streaming != Some(true) {
        let raw = pending.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&pending);
        let value = serde_json::from_slice::<Value>(raw);
        if !(200..300).contains(&info.status) {
            let shape = value.as_ref().map(response_shape).unwrap_or_else(|_| {
                if raw.iter().all(u8::is_ascii_whitespace) {
                    "empty"
                } else if raw.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') {
                    "html"
                } else {
                    "non_json"
                }
            });
            return Err(info.error(
                failure(
                    info.status,
                    value.as_ref().unwrap_or(&Value::Null),
                    &info.request,
                ),
                shape,
            ));
        }
        let value = value.map_err(|_| {
            if raw.iter().all(u8::is_ascii_whitespace) {
                info.error("ChatGPT 返回了空响应", "empty")
            } else if raw.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') {
                info.error(
                    "ChatGPT 返回了网页，可能是网络防护或代理页面，请检查连接",
                    "html",
                )
            } else {
                info.error("ChatGPT 返回的内容无法识别为回答或事件流", "non_json")
            }
        })?;
        reply = json_reply(&value, &info)?;
        progress(reply.clone());
    } else {
        if !completed && !pending.is_empty() {
            completed =
                frame(&pending, &mut reply, &info.request).map_err(|e| info.error(e, "sse"))?;
            if completed {
                progress(reply.clone());
            }
        }
        ensure!(
            completed,
            "{}",
            info.error("ChatGPT 连接已中断，回答未完整生成，请重试", "sse")
        );
        ensure!(
            !reply.answer.trim().is_empty(),
            "{}",
            info.error("ChatGPT 没有返回回答正文", "sse")
        );
    }
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    Ok(reply)
}
pub(super) async fn models(client: &Client, cancel: &Cancel) -> Result<Snapshot> {
    let (profile, access) = client.access(cancel).await?;
    let response = crate::ai_chat::request(
        cancel,
        http(client)?
            .get(format!("{}/models", client.api))
            .bearer_auth(access)
            .send(),
    )
    .await?;
    let status = response.status().as_u16();
    let id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let value = read_json(response, cancel).await?;
    ensure!(
        (200..300).contains(&status),
        "{}",
        failure(status, &value, &id)
    );
    let models = value["models"]
        .as_array()
        .context("ChatGPT 没有返回模型目录")?
        .iter()
        .filter(|v| v["visibility"] == "list")
        .take(1000)
        .filter_map(|v| {
            let slug = v["slug"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 512)?;
            Some(Model {
                slug: slug.into(),
                display_name: v["display_name"]
                    .as_str()
                    .unwrap_or(slug)
                    .chars()
                    .take(200)
                    .collect(),
            })
        })
        .collect::<Vec<_>>();
    let _lock = storage::lock(&client.lib, cancel).await?;
    let mut state = snapshot(&client.lib)?;
    ensure!(
        state.active == profile.id,
        "ChatGPT 账户已切换，请重新读取模型"
    );
    let p = state
        .profiles
        .iter_mut()
        .find(|p| p.id == profile.id)
        .context("ChatGPT 账户不存在")?;
    ensure!(p.connected, "ChatGPT 已退出登录");
    if !models.iter().any(|m| m.slug == p.model) {
        p.model = models.first().map(|m| m.slug.clone()).unwrap_or_default();
    }
    p.models = models;
    storage::save(&client.lib, &state)?;
    Ok(state)
}
fn append(target: &mut String, delta: &str) -> Result<()> {
    ensure!(target.len() + delta.len() <= MAX_TEXT, "ChatGPT 回答过长");
    target.push_str(delta);
    Ok(())
}
fn frame(bytes: &[u8], reply: &mut ChatReply, request_id: &str) -> Result<bool> {
    let text = std::str::from_utf8(bytes)
        .context("ChatGPT 流数据编码无效")?
        .trim_start_matches('\u{feff}');
    let payload = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if payload.is_empty() {
        return Ok(false);
    }
    let value: Value = serde_json::from_str(&payload).context("ChatGPT 流事件无效")?;
    match value["type"].as_str().unwrap_or("") {
        "response.output_text.delta" => append(
            &mut reply.answer,
            value["delta"].as_str().context("ChatGPT 回答片段无效")?,
        )?,
        "response.reasoning_summary_text.delta" => {
            append(&mut reply.reasoning, value["delta"].as_str().unwrap_or(""))?
        }
        "response.failed" | "error" => {
            return Err(failure(
                200,
                if value["response"].is_object() {
                    &value["response"]
                } else {
                    &value
                },
                request_id,
            ));
        }
        "response.incomplete" => anyhow::bail!("ChatGPT 回答未完整生成，请重试"),
        "response.refusal.delta" => anyhow::bail!("ChatGPT 未能回答此问题，请调整问题后重试"),
        "response.completed" => {
            if value["response"].get("error").is_some_and(|v| !v.is_null())
                || value["response"]
                    .get("detail")
                    .is_some_and(|v| !v.is_null())
            {
                return Err(failure(200, &value["response"], request_id));
            }
            ensure!(
                value["response"]["status"] == "completed",
                "ChatGPT 没有确认回答完成"
            );
            let final_text = output_text(&value["response"])?;
            if !final_text.is_empty() {
                reply.answer = final_text;
            }
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}
pub(super) async fn stream(
    client: &Client,
    messages: Vec<Value>,
    cancel: &Cancel,
    progress: impl FnMut(ChatReply),
) -> Result<ChatReply> {
    let (profile, access) = client.access(cancel).await?;
    ensure!(
        !profile.model.is_empty(),
        "请在设置中读取 ChatGPT 模型并选择一个模型"
    );
    let mut instructions = String::new();
    let mut input = vec![];
    for message in messages {
        match message["role"].as_str() {
            Some("system" | "developer") => {
                if let Some(s) = message["content"].as_str() {
                    if !instructions.is_empty() {
                        instructions.push('\n');
                    }
                    instructions.push_str(s);
                }
            }
            Some("user" | "assistant") => input.push(message),
            _ => {}
        }
    }
    ensure!(!input.is_empty(), "请先输入问题");
    let body = json!({"model":profile.model,"instructions":instructions,"input":input,"store":false,"stream":true});
    let response = crate::ai_chat::request(
        cancel,
        http(client)?
            .post(format!("{}/responses", client.api))
            .bearer_auth(access)
            .header("Accept", "text/event-stream")
            .json(&body)
            .send(),
    )
    .await
    .map_err(network_error)?;
    receive(response, cancel, progress).await
}
