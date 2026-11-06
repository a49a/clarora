use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::atomic::Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Debug)]
pub(super) struct Expired(String);
impl std::fmt::Display for Expired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Expired {}
fn random() -> Result<String> {
    let mut v = [0; 32];
    SystemRandom::new()
        .fill(&mut v)
        .map_err(|_| anyhow::anyhow!("生成登录随机数失败"))?;
    Ok(B64.encode(v))
}
async fn limited(response: reqwest::Response, cancel: &Cancel) -> Result<Value> {
    responses::read_json(response, cancel).await
}
async fn metadata(client: &Client, cancel: &Cancel) -> Result<Value> {
    let response = crate::ai_chat::request(
        cancel,
        http(client)?
            .get(format!("{}/.well-known/openid-configuration", client.auth))
            .send(),
    )
    .await?;
    ensure!(
        response.status().is_success(),
        "无法读取 ChatGPT 登录服务配置，请稍后重试"
    );
    let value = limited(response, cancel).await?;
    ensure!(value["issuer"] == ISSUER, "ChatGPT 身份服务签发者不匹配");
    Ok(value)
}
fn endpoint(client: &Client, value: &Value, key: &str) -> Result<String> {
    let address = value[key].as_str().context("ChatGPT 身份服务配置不完整")?;
    let url = url::Url::parse(address)?;
    let base = url::Url::parse(&client.auth)?;
    ensure!(
        url.origin() == base.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "ChatGPT 身份服务端点不匹配"
    );
    Ok(address.into())
}
async fn validate(
    client: &Client,
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
    cancel: &Cancel,
) -> Result<Value> {
    ensure!(token.len() <= 64 * 1024, "ChatGPT 身份凭证过长");
    let parts = token.split('.').collect::<Vec<_>>();
    ensure!(parts.len() == 3, "ChatGPT 身份凭证格式无效");
    let header: Value = serde_json::from_slice(&B64.decode(parts[0])?)?;
    let alg = header["alg"].as_str().unwrap_or("");
    ensure!(
        ["RS256", "ES256"].contains(&alg),
        "ChatGPT 身份签名算法不受支持"
    );
    let kid = header["kid"]
        .as_str()
        .context("ChatGPT 身份签名缺少密钥标识")?;
    let meta = metadata(client, cancel).await?;
    let jwks = endpoint(client, &meta, "jwks_uri")?;
    let response = crate::ai_chat::request(cancel, http(client)?.get(jwks).send()).await?;
    ensure!(response.status().is_success(), "读取 ChatGPT 签名公钥失败");
    let keys = limited(response, cancel).await?;
    let key = keys["keys"]
        .as_array()
        .context("ChatGPT 签名公钥无效")?
        .iter()
        .find(|k| k["kid"].as_str() == Some(kid))
        .context("未找到 ChatGPT 身份签名公钥")?;
    ensure!(
        key.get("use").is_none_or(|v| v == "sig") && key.get("alg").is_none_or(|v| v == alg),
        "ChatGPT 身份公钥用途不匹配"
    );
    let signed = format!("{}.{}", parts[0], parts[1]);
    let sig = B64.decode(parts[2])?;
    let field = |name: &str| -> Result<Vec<u8>> {
        Ok(B64.decode(key[name].as_str().context("ChatGPT 签名公钥字段缺失")?)?)
    };
    let verified = if alg == "RS256" && key["kty"] == "RSA" {
        let n = field("n")?;
        let e = field("e")?;
        signature::RsaPublicKeyComponents { n: &n, e: &e }.verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            signed.as_bytes(),
            &sig,
        )
    } else if alg == "ES256" && key["kty"] == "EC" && key["crv"] == "P-256" {
        let x = field("x")?;
        let y = field("y")?;
        ensure!(x.len() == 32 && y.len() == 32, "ChatGPT 签名公钥长度错误");
        let mut point = vec![4];
        point.extend(x);
        point.extend(y);
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point)
            .verify(signed.as_bytes(), &sig)
    } else {
        anyhow::bail!("ChatGPT 身份公钥类型不匹配")
    };
    verified.map_err(|_| anyhow::anyhow!("ChatGPT 身份签名验证失败"))?;
    let claims: Value = serde_json::from_slice(&B64.decode(parts[1])?)?;
    ensure!(claims["iss"] == ISSUER, "ChatGPT 身份签发者不匹配");
    let audience = &claims["aud"];
    let valid = audience.as_str() == Some(client_id)
        || audience
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(client_id)));
    ensure!(valid, "ChatGPT 身份凭证不属于此应用注册");
    if audience.as_array().is_some_and(|a| a.len() > 1) || claims.get("azp").is_some() {
        ensure!(claims["azp"] == client_id, "ChatGPT 身份授权方不匹配");
    }
    let now = chrono::Utc::now().timestamp();
    ensure!(
        claims["exp"].as_i64().is_some_and(|v| v > now),
        "ChatGPT 身份凭证已过期"
    );
    ensure!(
        claims
            .get("nbf")
            .is_none_or(|v| v.as_i64().is_some_and(|n| n <= now + 30)),
        "ChatGPT 身份凭证尚未生效"
    );
    ensure!(
        claims["sub"].as_str().is_some_and(|s| !s.is_empty()),
        "ChatGPT 身份缺少账户标识"
    );
    if let Some(nonce) = nonce {
        ensure!(claims["nonce"] == nonce, "ChatGPT 登录校验失败（nonce）");
    }
    if let Some(subject) = subject.filter(|s| !s.is_empty()) {
        ensure!(
            claims["sub"] == subject,
            "登录的 ChatGPT 账户与所选账户不同，请通过添加账户登录"
        );
    }
    Ok(claims)
}
async fn exchange(
    client: &Client,
    form: &[(&str, &str)],
    cancel: &Cancel,
    refresh: bool,
) -> Result<Value> {
    let response = crate::ai_chat::request(
        cancel,
        http(client)?
            .post(format!("{}/api/accounts/oauth/token", client.auth))
            .form(form)
            .send(),
    )
    .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let value = limited(response, cancel).await;
    ensure!(!cancel.load(Ordering::SeqCst), "已取消 ChatGPT 操作");
    if !status.is_success() {
        let value = value.unwrap_or(Value::Null);
        let code = login_error::code(&value);
        let error = login_error::failure(status.as_u16(), &headers, &value, refresh);
        if refresh
            && [
                "invalid_grant",
                "invalid_refresh_token",
                "token_expired",
                "refresh_token_expired",
                "refresh_token_invalidated",
                "refresh_token_reused",
            ]
            .contains(&code)
        {
            return Err(Expired(error.to_string()).into());
        }
        return Err(error);
    }
    value
}
fn tokens(
    value: &Value,
    client_id: &str,
    subject: &str,
    previous: Option<&Session>,
) -> Result<Session> {
    ensure!(
        value["token_type"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("Bearer")),
        "ChatGPT 返回的凭证类型无效"
    );
    let access = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("ChatGPT 没有返回访问凭证")?;
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("ChatGPT 没有返回续期凭证，请重新授权离线访问")?;
    let expiry = value["expires_in"]
        .as_i64()
        .filter(|s| *s > 0 && *s <= 86400)
        .context("ChatGPT 凭证有效期无效")?;
    let scopes = if let Some(s) = value["scope"].as_str() {
        s.split_whitespace().map(str::to_string).collect()
    } else {
        previous.map(|s| s.scopes.clone()).unwrap_or_default()
    };
    let id = value["id_token"]
        .as_str()
        .map(str::to_string)
        .or_else(|| previous.map(|s| s.id_token.clone()))
        .context("ChatGPT 没有返回身份凭证")?;
    Ok(Session {
        client_id: client_id.into(),
        subject: subject.into(),
        access_token: access.into(),
        refresh_token: refresh.into(),
        id_token: id,
        scopes,
        expires_at: chrono::Utc::now().timestamp() + expiry,
    })
}
async fn callback(
    listener: &TcpListener,
    state: &str,
    cancel: &Cancel,
) -> Result<BTreeMap<String, String>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    loop {
        let accepted = tokio::select! {biased;_=crate::ai_chat::cancelled(cancel)=>anyhow::bail!("已取消 ChatGPT 登录"),_=tokio::time::sleep_until(deadline)=>anyhow::bail!("ChatGPT 登录等待超时，请重试"),v=listener.accept()=>v?};
        let (mut socket, _) = accepted;
        let mut bytes = vec![];
        let mut block = [0; 1024];
        let read = async {
            loop {
                let n = socket.read(&mut block).await?;
                ensure!(n > 0, "callback interrupted");
                bytes.extend_from_slice(&block[..n]);
                ensure!(bytes.len() <= 16 * 1024, "callback too large");
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Ok::<(), anyhow::Error>(())
        };
        let read = tokio::select! {biased;_=crate::ai_chat::cancelled(cancel)=>anyhow::bail!("已取消 ChatGPT 登录"),v=tokio::time::timeout(Duration::from_secs(2),read)=>v};
        if !matches!(read, Ok(Ok(()))) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut first = text.lines().next().unwrap_or("").split_whitespace();
        let method = first.next().unwrap_or("");
        let target = first.next().unwrap_or("");
        let parsed = url::Url::parse(&format!("http://127.0.0.1{target}"));
        let mut params = BTreeMap::new();
        let mut valid = method == "GET" && target.starts_with("/auth/callback?");
        if let Ok(url) = parsed {
            valid &= url.path() == "/auth/callback";
            for (k, v) in url.query_pairs() {
                if params.insert(k.into_owned(), v.into_owned()).is_some() {
                    valid = false;
                }
            }
        } else {
            valid = false;
        }
        valid &= params.get("state").is_some_and(|v| v == state);
        let body = if valid {
            "<!doctype html><meta charset=utf-8><title>Clarora</title><p>已收到登录回调，此页面不代表登录成功。请回到 Clarora 查看最终登录结果。</p>"
        } else {
            "Invalid callback"
        };
        let response = format!(
            "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            if valid { "200 OK" } else { "400 Bad Request" },
            body.len()
        );
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            socket.write_all(response.as_bytes()),
        )
        .await;
        if valid {
            return Ok(params);
        }
    }
}
pub(super) async fn sign_in(
    client: &Client,
    profile: Option<&str>,
    cancel: &Cancel,
    mut open: impl FnMut(&str) -> Result<()>,
) -> Result<Snapshot> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let mut pending = {
        let _lock = storage::lock(&client.lib, cancel).await?;
        let mut state = snapshot(&client.lib)?;
        if state.host_id.is_empty() {
            state.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            storage::save(&client.lib, &state)?;
        }
        let profile = if let Some(id) = profile {
            state
                .profiles
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .context("ChatGPT 账户不存在")?
        } else {
            ensure!(state.profiles.len() < 100, "保存的 ChatGPT 账户过多");
            Profile {
                id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            }
        };
        (state.host_id, profile)
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener.local_addr()?.port()
    );
    let state = random()?;
    let nonce = random()?;
    let verifier = random()?;
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let new = pending.1.client_id.is_empty();
    let mut address = url::Url::parse(&format!("{}/api/accounts/authorize", client.auth))?;
    {
        let mut pairs = address.query_pairs_mut();
        pairs
            .append_pair(
                "client_id",
                if new {
                    "dynamic_agent_client"
                } else {
                    &pending.1.client_id
                },
            )
            .append_pair("ext_agent_host_id", &pending.0)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair(
                "scope",
                "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            )
            .append_pair("resource", RESOURCE)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge);
        if new {
            pairs.append_pair("agent_name_hint", "Clarora");
        } else {
            if let Ok(Some(session)) = storage::session(&client.lib, &pending.1) {
                if !session.id_token.is_empty() {
                    pairs.append_pair("id_token_hint", &session.id_token);
                }
            }
            if !pending.1.email.is_empty() {
                pairs.append_pair("login_hint", &pending.1.email);
            }
            if !pending.1.sharing {
                pairs.append_pair("prompt", "consent");
            }
        }
    }
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    open(address.as_str())?;
    let returned = callback(&listener, &state, cancel).await?;
    if returned.contains_key("error") {
        anyhow::bail!("ChatGPT 授权未完成或已被拒绝，可重新点击登录");
    }
    let issued = returned
        .get("client_id")
        .cloned()
        .unwrap_or_else(|| pending.1.client_id.clone());
    ensure!(
        !issued.is_empty() && issued != "dynamic_agent_client" && issued.len() <= 1024,
        "ChatGPT 注册未完成，没有返回应用标识"
    );
    ensure!(
        new || issued == pending.1.client_id,
        "ChatGPT 返回的应用注册不匹配"
    );
    let code = returned
        .get("code")
        .filter(|s| !s.is_empty())
        .context("ChatGPT 没有返回授权码")?;
    pending.1.client_id = issued.clone();
    // Retain issued registration before exchanging a one-time code, but never activate it yet.
    {
        let _lock = storage::lock(&client.lib, cancel).await?;
        let mut state = snapshot(&client.lib)?;
        if !state.profiles.iter().any(|p| p.id == pending.1.id) {
            state.profiles.push(pending.1.clone());
            storage::save(&client.lib, &state)?;
        }
    }
    let value = exchange(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &issued),
            ("code", code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("resource", RESOURCE),
        ],
        cancel,
        false,
    )
    .await?;
    let id = value["id_token"]
        .as_str()
        .context("ChatGPT 没有返回身份凭证")?;
    let claims = validate(
        client,
        id,
        &issued,
        Some(&nonce),
        Some(&pending.1.subject),
        cancel,
    )
    .await?;
    let p = &mut pending.1;
    p.subject = claims["sub"]
        .as_str()
        .context("ChatGPT 账户标识缺失")?
        .into();
    p.email = claims["email"].as_str().unwrap_or("").into();
    let session = tokens(&value, &issued, &p.subject, None)?;
    p.sharing = session.scopes.iter().any(|s| s == DIRECT);
    p.connected = true;
    {
        let _lock = storage::lock(&client.lib, cancel).await?;
        let mut state = snapshot(&client.lib)?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        storage::save_session(&client.lib, p, &session)?;
        let saved = state
            .profiles
            .iter_mut()
            .find(|v| v.id == p.id)
            .context("ChatGPT 注册已移除")?;
        *saved = p.clone();
        state.active = p.id.clone();
        // Choosing ChatGPT must never fall back to separately billed API keys,
        // including when the user has not granted plan usage yet.
        state.enabled = true;
        storage::save(&client.lib, &state)?;
    }
    if p.sharing {
        let _ = responses::models(client, cancel).await;
    }
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    snapshot(&client.lib)
}
pub(super) async fn refresh(client: &Client, old: &Session, cancel: &Cancel) -> Result<Session> {
    let value = exchange(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &old.client_id),
            ("refresh_token", &old.refresh_token),
            ("resource", RESOURCE),
        ],
        cancel,
        true,
    )
    .await?;
    if let Some(id) = value["id_token"].as_str() {
        validate(client, id, &old.client_id, None, Some(&old.subject), cancel).await?;
    }
    tokens(&value, &old.client_id, &old.subject, Some(old))
}
pub(super) async fn sign_out(client: &Client, cancel: &Cancel) -> Result<(Snapshot, bool)> {
    let _lock = storage::lock(&client.lib, cancel).await?;
    let mut state = snapshot(&client.lib)?;
    let p = state
        .current()
        .cloned()
        .context("没有选中的 ChatGPT 账户")?;
    let session = storage::session(&client.lib, &p);
    let mut revoked = matches!(session, Ok(None));
    // Local sign-out must remain possible when the OS vault is locked/damaged.
    if let Ok(Some(session)) = session {
        for attempt in 0..2 {
            let revoke = async {
                let meta = metadata(client, cancel).await?;
                let endpoint = endpoint(client, &meta, "revocation_endpoint")?;
                let response = crate::ai_chat::request(
                    cancel,
                    http(client)?
                        .post(endpoint)
                        .form(&[
                            ("token", session.refresh_token.as_str()),
                            ("token_type_hint", "refresh_token"),
                            ("client_id", session.client_id.as_str()),
                        ])
                        .send(),
                )
                .await?;
                Ok::<bool, anyhow::Error>(response.status().as_u16() == 200)
            };
            revoked = tokio::time::timeout(Duration::from_secs(12), revoke)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
            if revoked {
                break;
            }
            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
    storage::clear_session(&client.lib, &p)?;
    if let Some(p) = state.profiles.iter_mut().find(|v| v.id == p.id) {
        p.connected = false;
    }
    storage::save(&client.lib, &state)?;
    Ok((state, revoked))
}
