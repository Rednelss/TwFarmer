use crate::config::Config;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path, time::Duration};
use tokio::time::{Instant, sleep};

const DEVICE: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE: &str = "https://id.twitch.tv/oauth2/validate";

#[derive(Serialize, Deserialize)]
pub struct Auth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_id: String,
    pub device_id: String,
}

#[derive(Deserialize)]
pub struct Identity {
    pub client_id: String,
    pub login: String,
    pub user_id: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug)]
pub struct SessionExpired;
impl std::fmt::Display for SessionExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Сессия Twitch недействительна. Выполните twfarmer login."
        )
    }
}
impl std::error::Error for SessionExpired {}

pub fn read(path: &Path) -> Result<Auth> {
    let bytes = fs::read(path).context("Нет сохранённой сессии. Выполните twfarmer login.")?;
    serde_json::from_slice(&bytes).context("Файл сессии повреждён. Выполните twfarmer login.")
}

pub fn save(path: &Path, auth: &Auth) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // tempfile creates files with mode 0600 on Unix; rename replaces atomically.
    let mut file =
        tempfile::NamedTempFile::new_in(parent).context("Не удалось создать файл сессии")?;
    file.write_all(&serde_json::to_vec(auth)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow::anyhow!("Не удалось сохранить файл сессии"))?;
    Ok(())
}

pub async fn validate(client: &Client, auth: &Auth) -> Result<Identity> {
    validate_at(client, auth, VALIDATE).await
}

async fn validate_at(client: &Client, auth: &Auth, endpoint: &str) -> Result<Identity> {
    let response = client
        .get(endpoint)
        .header("Authorization", format!("OAuth {}", auth.access_token))
        .send()
        .await
        .map_err(|e| e.without_url())
        .context("Проверка сессии Twitch")?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(SessionExpired.into());
    }
    crate::twitch::check_status(response.status(), "Проверка сессии")?;
    let identity: Identity = response
        .json()
        .await
        .context("Неверный ответ проверки сессии")?;
    ensure!(
        identity.client_id == auth.client_id,
        "client_id токена не совпадает с сохранённой сессией"
    );
    ensure!(
        !identity.user_id.is_empty() && !identity.login.is_empty(),
        "Нужен пользовательский токен Twitch"
    );
    Ok(identity)
}

pub async fn ensure_valid(client: &Client, auth: &mut Auth, path: &Path) -> Result<Identity> {
    ensure_valid_at(client, auth, path, VALIDATE, TOKEN).await
}

async fn ensure_valid_at(
    client: &Client,
    auth: &mut Auth,
    path: &Path,
    validate_endpoint: &str,
    token_endpoint: &str,
) -> Result<Identity> {
    match validate_at(client, auth, validate_endpoint).await {
        // TV tokens can report zero lifetime while /validate confirms they are valid.
        // Keep checking them periodically; refresh only after a 401 or a positive
        // lifetime below the refresh threshold.
        Ok(identity) if identity.expires_in == 0 || identity.expires_in > 300 => Ok(identity),
        Ok(identity) if auth.refresh_token.is_none() => Ok(identity),
        Ok(_) => {
            refresh_at(client, auth, path, token_endpoint).await?;
            validate_at(client, auth, validate_endpoint).await
        }
        Err(error) if error.is::<SessionExpired>() => {
            refresh_at(client, auth, path, token_endpoint).await?;
            validate_at(client, auth, validate_endpoint).await
        }
        Err(error) => Err(error),
    }
}

async fn refresh_at(client: &Client, auth: &mut Auth, path: &Path, endpoint: &str) -> Result<()> {
    let refresh = auth.refresh_token.as_deref().ok_or(SessionExpired)?;
    let response = client
        .post(endpoint)
        .form(&[
            ("client_id", auth.client_id.as_str()),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| e.without_url())
        .context("Обновление сессии")?;
    if matches!(
        response.status(),
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(SessionExpired.into());
    }
    crate::twitch::check_status(response.status(), "Обновление сессии")?;
    let tokens: Tokens = response
        .json()
        .await
        .context("Неверный ответ обновления сессии")?;
    auth.access_token = tokens.access_token;
    if tokens.refresh_token.is_some() {
        auth.refresh_token = tokens.refresh_token;
    }
    save(path, auth)
}

pub async fn login(client: &Client, config: &Config, path: &Path) -> Result<Identity> {
    let device_id = uuid::Uuid::new_v4().simple().to_string();
    let response = client
        .post(DEVICE)
        .header("Origin", "https://android.tv.twitch.tv")
        .header("Referer", "https://android.tv.twitch.tv/")
        .header("Client-ID", &config.client_id)
        .header("X-Device-ID", &device_id)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("scopes", &config.oauth_scopes.join(" ")),
        ])
        .send()
        .await
        .map_err(|e| e.without_url())
        .context("Запрос кода входа")?;
    crate::twitch::check_status(
        response.status(),
        "Код входа: проверьте client_id и oauth_scopes",
    )?;
    let device: Device = response
        .json()
        .await
        .context("Неверный ответ запроса кода")?;
    let uri = reqwest::Url::parse(&device.verification_uri)?;
    ensure!(
        uri.scheme() == "https" && matches!(uri.host_str(), Some("twitch.tv" | "www.twitch.tv")),
        "Неверный адрес авторизации Twitch"
    );
    println!("Откройте в браузере: {}", device.verification_uri);
    println!(
        "Код входа: {}. Подтвердите вход на сайте Twitch.",
        device.user_code
    );
    println!("Ожидаю подтверждение; Ctrl+C — отмена.");
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval.clamp(1, 60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "Код истёк. Выполните twfarmer login ещё раз."
        );
        sleep(Duration::from_secs(interval).min(remaining)).await;
        ensure!(
            Instant::now() < deadline,
            "Код истёк. Выполните twfarmer login ещё раз."
        );
        let response = client
            .post(TOKEN)
            .header("Origin", "https://android.tv.twitch.tv")
            .header("Client-ID", &config.client_id)
            .header("X-Device-ID", &device_id)
            .form(&[
                ("client_id", config.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Ожидание входа")?;
        let status = response.status();
        if status.is_success() {
            let tokens: Tokens = response.json().await.context("Неверный ответ входа")?;
            let auth = Auth {
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                client_id: config.client_id.clone(),
                device_id,
            };
            let identity = validate(client, &auth).await?;
            save(path, &auth)?;
            return Ok(identity);
        }
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            interval = (interval + 5).min(60);
            continue;
        }
        let body: serde_json::Value = response
            .json()
            .await
            .context("Неверный ответ ожидания входа")?;
        match poll_action(&body) {
            Poll::Pending => {}
            Poll::Slower => interval = (interval + 5).min(60),
            Poll::Denied => bail!("Вход отклонён на сайте Twitch"),
            Poll::Expired => bail!("Код истёк. Выполните twfarmer login ещё раз."),
            Poll::Unknown => {
                bail!("Twitch отклонил вход (HTTP {status}); проверьте client_id и oauth_scopes")
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Poll {
    Pending,
    Slower,
    Denied,
    Expired,
    Unknown,
}
fn poll_action(body: &serde_json::Value) -> Poll {
    let message = body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("message").and_then(|v| v.as_str()))
        .unwrap_or("");
    match message {
        "authorization_pending" | "authorization pending" => Poll::Pending,
        "slow_down" | "slow down" => Poll::Slower,
        "access_denied" | "access denied" => Poll::Denied,
        "expired_token" | "invalid device code" => Poll::Expired,
        _ => Poll::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn mock_oauth(
        responses: Vec<(&'static str, u16, serde_json::Value)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut paths = Vec::new();
            for (expected_path, status, body) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "OAuth request did not arrive"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                let path = loop {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&request[..end]).unwrap();
                        let body_size = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|size| size.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + body_size {
                            break header
                                .lines()
                                .next()
                                .unwrap()
                                .split_whitespace()
                                .nth(1)
                                .unwrap()
                                .to_owned();
                        }
                    }
                };
                assert_eq!(path, expected_path);
                paths.push(path);
                let body = body.to_string();
                write!(
                    socket,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            paths
        });
        (address, server)
    }

    fn test_auth() -> Auth {
        Auth {
            access_token: "test-access-token".into(),
            refresh_token: Some("test-refresh-token".into()),
            client_id: "test-client".into(),
            device_id: "test-device".into(),
        }
    }

    fn identity_body(expires_in: u64) -> serde_json::Value {
        json!({"client_id":"test-client", "login":"test-user", "user_id":"123", "expires_in":expires_in})
    }

    #[tokio::test]
    async fn valid_zero_lifetime_does_not_refresh_or_rewrite_session() {
        let (endpoint, server) = mock_oauth(vec![("/validate", 200, identity_body(0))]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.auth.json");
        let mut auth = test_auth();
        save(&path, &auth).unwrap();
        let saved = fs::read(&path).unwrap();
        let identity = ensure_valid_at(
            &Client::new(),
            &mut auth,
            &path,
            &format!("{endpoint}/validate"),
            &format!("{endpoint}/token"),
        )
        .await
        .unwrap();
        assert_eq!(identity.login, "test-user");
        assert_eq!(identity.expires_in, 0);
        assert_eq!(auth.access_token, "test-access-token");
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(server.join().unwrap(), ["/validate"]);
    }

    #[tokio::test]
    async fn refreshes_expiring_and_rejected_tokens() {
        for (status, validation) in [
            (200, identity_body(120)),
            (401, json!({"message":"invalid access token"})),
        ] {
            let (endpoint, server) = mock_oauth(vec![
                ("/validate", status, validation),
                (
                    "/token",
                    200,
                    json!({"access_token":"renewed-access", "refresh_token":"renewed-refresh"}),
                ),
                ("/validate", 200, identity_body(3600)),
            ]);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("session.auth.json");
            let mut auth = test_auth();
            let identity = ensure_valid_at(
                &Client::new(),
                &mut auth,
                &path,
                &format!("{endpoint}/validate"),
                &format!("{endpoint}/token"),
            )
            .await
            .unwrap();
            assert_eq!(identity.expires_in, 3600);
            assert_eq!(read(&path).unwrap().access_token, "renewed-access");
            assert_eq!(auth.refresh_token.as_deref(), Some("renewed-refresh"));
            assert_eq!(server.join().unwrap(), ["/validate", "/token", "/validate"]);
        }
    }

    #[test]
    fn handles_twitch_and_oauth_poll_responses() {
        assert_eq!(
            poll_action(&json!({"message":"authorization_pending"})),
            Poll::Pending
        );
        assert_eq!(poll_action(&json!({"error":"slow_down"})), Poll::Slower);
        assert_eq!(poll_action(&json!({"error":"access_denied"})), Poll::Denied);
        assert_eq!(
            poll_action(&json!({"message":"invalid device code"})),
            Poll::Expired
        );
        assert_eq!(
            poll_action(&json!({"message":"invalid client"})),
            Poll::Unknown
        );
    }
    #[test]
    fn session_is_saved_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.auth.json");
        let auth = Auth {
            access_token: "test-only".into(),
            refresh_token: None,
            client_id: "client".into(),
            device_id: "device".into(),
        };
        save(&path, &auth).unwrap();
        assert_eq!(read(&path).unwrap().access_token, "test-only");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        save(
            &path,
            &Auth {
                access_token: "updated".into(),
                ..auth
            },
        )
        .unwrap();
        assert_eq!(read(&path).unwrap().access_token, "updated");
    }
}
