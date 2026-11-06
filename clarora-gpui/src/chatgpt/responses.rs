use super::*;
use serde_json::json;
use std::sync::atomic::Ordering;
const MAX_TEXT: usize = 4 * 1024 * 1024;
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
        .or_else(|| value["code"].as_str())
        .unwrap_or("");
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
pub(super) async fn models(client: &Client, cancel: &Cancel) -> Result<Snapshot> {
    let (profile, access) = client.access(cancel).await?;
    let response = crate::ai_chat::request(
        cancel,
        http()?
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
    let text = std::str::from_utf8(bytes).context("ChatGPT 流数据编码无效")?;
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
            ensure!(
                value["response"]["status"] == "completed",
                "ChatGPT 没有确认回答完成"
            );
            if reply.answer.is_empty() {
                if let Some(output) = value["response"]["output"].as_array() {
                    for item in output {
                        if item["type"] == "message" && item["role"] == "assistant" {
                            if let Some(content) = item["content"].as_array() {
                                for part in content {
                                    if part["type"] == "output_text" {
                                        append(
                                            &mut reply.answer,
                                            part["text"].as_str().unwrap_or(""),
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                }
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
    mut progress: impl FnMut(ChatReply),
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
    let mut response = crate::ai_chat::request(
        cancel,
        http()?
            .post(format!("{}/responses", client.api))
            .bearer_auth(access)
            .header("Accept", "text/event-stream")
            .json(&body)
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
    if !(200..300).contains(&status) {
        let value = read_json(response, cancel).await.unwrap_or(Value::Null);
        return Err(failure(status, &value, &id));
    }
    ensure!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream")),
        "ChatGPT 没有返回流式回答"
    );
    let mut pending = vec![];
    let mut reply = ChatReply::default();
    let mut completed = false;
    while let Some(chunk) = crate::ai_chat::request(cancel, response.chunk()).await? {
        ensure!(
            pending.len() + chunk.len() <= 8 * 1024 * 1024,
            "ChatGPT 流事件过长"
        );
        pending.extend_from_slice(&chunk);
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
            completed = frame(&pending[..i], &mut reply, &id)?;
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
    ensure!(completed, "ChatGPT 连接已中断，回答未完整生成，请重试");
    ensure!(!reply.answer.trim().is_empty(), "ChatGPT 没有返回回答");
    Ok(reply)
}
