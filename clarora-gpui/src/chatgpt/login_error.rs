//! A bounded, non-secret diagnosis of OAuth credential exchange failures.
use super::*;
use reqwest::header::HeaderMap;

pub(super) fn code(value: &Value) -> &str {
    value["error"]
        .as_str()
        .or_else(|| value["error"]["code"].as_str())
        .or_else(|| value["code"].as_str())
        .unwrap_or("")
}
fn identifier(value: &str) -> String {
    if value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:".contains(c))
    {
        value.into()
    } else {
        String::new()
    }
}
pub(super) fn failure(
    status: u16,
    headers: &HeaderMap,
    value: &Value,
    refresh: bool,
) -> anyhow::Error {
    let stage = if refresh {
        "续期登录凭证"
    } else {
        "换取登录凭证"
    };
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    let shape = if value["error"].is_object() {
        "error_object"
    } else if value["error"].is_string() {
        "error_string"
    } else if value.get("detail").is_some() {
        "detail"
    } else if !value.is_null() {
        "json"
    } else if header("content-type").contains("text/html") {
        "html"
    } else {
        "non_json"
    };
    let description = match code(value) {
        "unsupported_country_region_territory"
        | "unsupported_country"
        | "country_not_supported" => {
            "OpenAI 不支持当前请求的服务地区。请检查所用网络及服务可用地区；反复登录无法解除该限制。"
        }
        "invalid_client" | "unauthorized_client" => {
            "OpenAI 未接受 Clarora 的应用注册。请复制错误详情用于排查；反复登录无法修复注册配置。"
        }
        "invalid_grant" if !refresh => {
            "授权码已失效，请点击 Continue with ChatGPT 重新授权，将复用已保存的应用注册。"
        }
        "invalid_grant"
        | "invalid_refresh_token"
        | "token_expired"
        | "refresh_token_expired"
        | "refresh_token_invalidated"
        | "refresh_token_reused" => "ChatGPT 登录凭证已失效或被撤销，请重新登录。",
        "access_denied"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => {
            "OpenAI 未接受本次应用授权。请复制错误详情检查账户及应用授权权限。"
        }
        _ => match status {
            403 if shape == "html" || header("cf-mitigated") == "challenge" => {
                "登录服务返回了网络防护或代理页面。请核对 Clarora 与浏览器的网络设置，以及服务可用地区。"
            }
            403 => "OpenAI 拒绝了本次登录请求，具体限制尚无法确定。请复制错误详情进一步排查。",
            429 => "登录请求过于频繁，请稍后重试。",
            500..=599 => "OpenAI 登录服务暂时不可用，已保存的账户注册会保留，请稍后重试。",
            _ => "OpenAI 未完成本次登录操作。请复制错误详情进一步排查。",
        },
    };
    // Never include error_description, detail, raw HTML, callback query or tokens.
    anyhow::anyhow!(
        "{stage}失败（HTTP {status}）：{description}\n诊断：阶段={stage}; HTTP {status}; endpoint=/api/accounts/oauth/token; code={}; response={shape}; request={}; edge={}",
        identifier(code(value)),
        identifier(header("x-request-id")),
        identifier(header("cf-ray"))
    )
}
