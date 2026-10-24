use crate::domain::{AppError, AppResult, ErrorCode};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;
use zeroize::Zeroizing;

pub struct Tokens {
    pub access: Zeroizing<String>,
    pub refresh: Option<Zeroizing<String>>,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
}

fn random_url_safe() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_url(
    base: &Url,
    redirect: &str,
    role: &str,
    state: &str,
    verifier: &str,
) -> AppResult<Url> {
    let mut url = base
        .join("oauth/authorize")
        .map_err(|_| AppError::invalid("Invalid Snowflake account URL"))?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    url.query_pairs_mut()
        .append_pair("client_id", "LOCAL_APPLICATION")
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect)
        .append_pair("state", state)
        .append_pair("scope", &format!("refresh_token session:role:{role}"))
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

async fn callback(listener: TcpListener, state: &str) -> AppResult<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let (mut stream, peer) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| AppError::new(ErrorCode::Cancelled, "Snowflake sign-in timed out"))?
            .map_err(|_| {
                AppError::new(
                    ErrorCode::ConnectionFailed,
                    "Could not receive Snowflake sign-in",
                )
            })?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut request = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        while request.len() < 8192 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tokio::time::timeout_at(deadline, stream.read(&mut chunk))
                .await
                .map_err(|_| AppError::new(ErrorCode::Cancelled, "Snowflake sign-in timed out"))?
                .map_err(|_| AppError::invalid("Invalid browser callback"))?;
            if n == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..n]);
        }
        let line = String::from_utf8_lossy(&request);
        let Some(path) = line
            .lines()
            .next()
            .and_then(|first| first.strip_prefix("GET "))
            .and_then(|rest| rest.split_once(' ').map(|v| v.0))
        else {
            continue;
        };
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{path}")) else {
            continue;
        };
        if url.path() != "/" {
            continue;
        }
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        if params.get("state").map(String::as_str) != Some(state) {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let code = params.get("code").filter(|code| !code.is_empty()).cloned();
        let body = if code.is_some() {
            "Snowflake sign-in complete. Return to Joinery."
        } else {
            "Snowflake sign-in was not completed. Return to Joinery."
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return code
            .ok_or_else(|| AppError::new(ErrorCode::Cancelled, "Snowflake sign-in was cancelled"));
    }
}

pub async fn sign_in(client: &reqwest::Client, base: &Url, role: &str) -> AppResult<Tokens> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "Could not open local Snowflake sign-in callback",
            )
        })?;
    let port = listener
        .local_addr()
        .map_err(|_| AppError::internal("Missing sign-in callback port"))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let state = random_url_safe();
    let verifier = Zeroizing::new(random_url_safe());
    let url = authorize_url(base, &redirect, role, &state, &verifier)?;
    tokio::task::spawn_blocking(move || webbrowser::open(url.as_str()))
        .await
        .map_err(|_| AppError::internal("Could not launch the system browser"))?
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "Could not launch the system browser for Snowflake sign-in",
            )
        })?;
    let code = Zeroizing::new(callback(listener, &state).await?);
    token_request(
        client,
        base,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", "LOCAL_APPLICATION"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
        ],
    )
    .await
}

pub async fn refresh(client: &reqwest::Client, base: &Url, token: &str) -> AppResult<Tokens> {
    token_request(
        client,
        base,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", "LOCAL_APPLICATION"),
            ("refresh_token", token),
        ],
    )
    .await
}

async fn token_request(
    client: &reqwest::Client,
    base: &Url,
    form: &[(&str, &str)],
) -> AppResult<Tokens> {
    let url = base
        .join("oauth/token-request")
        .map_err(|_| AppError::invalid("Invalid Snowflake account URL"))?;
    let mut response = client.post(url).form(form).send().await.map_err(|_| {
        AppError::new(
            ErrorCode::ConnectionFailed,
            "Snowflake token request failed",
        )
    })?;
    if !response.status().is_success() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Snowflake sign-in was rejected. Check the account, role, and local OAuth integration.",
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        AppError::new(
            ErrorCode::ConnectionFailed,
            "Snowflake token response was interrupted",
        )
    })? {
        if bytes.len().saturating_add(chunk.len()) > 64 * 1024 {
            return Err(AppError::new(
                ErrorCode::ConnectionFailed,
                "Snowflake token response was too large",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: TokenResponse = serde_json::from_slice(&bytes).map_err(|_| {
        AppError::new(
            ErrorCode::ConnectionFailed,
            "Snowflake returned an invalid token response",
        )
    })?;
    if !body.token_type.eq_ignore_ascii_case("Bearer")
        || body.access_token.is_empty()
        || body.expires_in == 0
    {
        return Err(AppError::new(
            ErrorCode::ConnectionFailed,
            "Snowflake returned an unusable token",
        ));
    }
    Ok(Tokens {
        access: Zeroizing::new(body.access_token),
        refresh: body.refresh_token.map(Zeroizing::new),
        expires_in: body.expires_in.min(3600),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;
    #[test]
    fn authorization_is_scoped_and_uses_pkce() {
        let base = Url::parse("https://org-account.snowflakecomputing.com/").unwrap();
        let url = authorize_url(
            &base,
            "http://127.0.0.1:1234",
            "READ_ONLY",
            "state",
            "verifier",
        )
        .unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["scope"], "refresh_token session:role:READ_ONLY");
        assert_eq!(query["redirect_uri"], "http://127.0.0.1:1234");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_ne!(query["code_challenge"], "verifier");
        assert_eq!(query["state"], "state");
    }
    #[tokio::test]
    async fn callback_rejects_wrong_state_before_accepting_code() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move { callback(listener, "expected").await });
        for (state, code) in [("wrong", "bad"), ("expected", "good")] {
            let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            let request =
                format!("GET /?state={state}&code={code} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
        }
        assert_eq!(task.await.unwrap().unwrap(), "good");
    }

    #[tokio::test]
    async fn token_exchange_uses_public_local_client_and_pkce_verifier() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /oauth/token-request HTTP/1.1"));
            assert!(request.contains("client_id=LOCAL_APPLICATION"));
            assert!(request.contains("code_verifier=verifier"));
            assert!(!request.contains("client_secret="));
            let body = r#"{"access_token":"access","refresh_token":"refresh","expires_in":600,"token_type":"Bearer"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let client = reqwest::Client::new();
        let base = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let tokens = token_request(
            &client,
            &base,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", "LOCAL_APPLICATION"),
                ("code_verifier", "verifier"),
            ],
        )
        .await
        .unwrap();
        assert_eq!(tokens.access.as_str(), "access");
        assert_eq!(tokens.refresh.as_ref().unwrap().as_str(), "refresh");
        server.await.unwrap();
    }
}
