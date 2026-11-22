use anyhow::{Context, Result, ensure};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

const TOKEN: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE: &str = "https://id.twitch.tv/oauth2/validate";
pub const NATIVE_LOGIN_URL: &str = "https://www.twitch.tv/login?client_id=kd1unb4b3q4t58fwlpcbzcbnm76a8fp&redirect_params=client_id%3Dkd1unb4b3q4t58fwlpcbzcbnm76a8fp%26redirect_uri%3Dhttps%253A%252F%252Fwww.twitch.tv%252F%26response_type%3Dtoken";

#[derive(Serialize, Deserialize)]
pub struct Auth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_id: String,
    pub device_id: String,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Deserialize)]
pub struct Identity {
    pub client_id: String,
    pub login: String,
    pub user_id: String,
    pub expires_in: u64,
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
            "{}",
            tf!(
                "The Twitch session is invalid. Run twfarmer login to sign in again.",
                "Сессия Twitch недействительна. Выполните twfarmer login для повторного входа."
            )
        )
    }
}
impl std::error::Error for SessionExpired {}

pub fn read(path: &Path) -> Result<Auth> {
    let bytes = fs::read(path).context(tr!(
        "No saved session. Add an account.",
        "Нет сохранённой сессии. Добавьте аккаунт."
    ))?;
    serde_json::from_slice(&bytes).context(tr!(
        "The session file is corrupted. Sign in to the account again.",
        "Файл сессии повреждён. Обновите вход в аккаунт."
    ))
}

pub fn save(path: &Path, auth: &Auth) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // tempfile creates files with mode 0600 on Unix; rename replaces atomically.
    let mut file = tempfile::NamedTempFile::new_in(parent).context(tr!(
        "Could not create the session file",
        "Не удалось создать файл сессии"
    ))?;
    file.write_all(&serde_json::to_vec(auth)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|_| {
        anyhow::anyhow!(
            "{}",
            tf!(
                "Could not save the session file",
                "Не удалось сохранить файл сессии"
            )
        )
    })?;
    Ok(())
}

pub fn sync_identity(auth: &mut Auth, identity: &Identity, path: &Path) -> Result<()> {
    if auth.login.as_deref() != Some(identity.login.as_str())
        || auth.user_id.as_deref() != Some(identity.user_id.as_str())
    {
        auth.login = Some(identity.login.clone());
        auth.user_id = Some(identity.user_id.clone());
        save(path, auth)?;
    }
    Ok(())
}

pub async fn validate(client: &Client, auth: &Auth) -> Result<Identity> {
    validate_at(client, auth, VALIDATE).await
}

async fn validate_at(client: &Client, auth: &Auth, endpoint: &str) -> Result<Identity> {
    let identity = validate_token_at(client, &auth.access_token, endpoint).await?;
    ensure!(
        identity.client_id == auth.client_id,
        "{}",
        tf!(
            "The token client_id does not match the saved session",
            "client_id токена не совпадает с сохранённой сессией"
        )
    );
    Ok(identity)
}

async fn validate_token_at(client: &Client, token: &str, endpoint: &str) -> Result<Identity> {
    let response = client
        .get(endpoint)
        .header("Authorization", format!("OAuth {token}"))
        .send()
        .await
        .map_err(|e| e.without_url())
        .context(tr!(
            "Validating the Twitch session",
            "Проверка сессии Twitch"
        ))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(SessionExpired.into());
    }
    crate::twitch::check_status(
        response.status(),
        tr!("Session validation", "Проверка сессии"),
    )?;
    let identity: Identity = response.json().await.context(tr!(
        "Invalid session validation response",
        "Неверный ответ проверки сессии"
    ))?;
    ensure!(
        !identity.user_id.is_empty()
            && !identity.login.is_empty()
            && !identity.client_id.is_empty(),
        "{}",
        tf!(
            "A Twitch user token is required",
            "Нужен пользовательский токен Twitch"
        )
    );
    Ok(identity)
}

fn token_input(input: &str) -> Result<String> {
    let invalid = || {
        anyhow::anyhow!(
            "{}",
            tf!(
                "Enter a Twitch access token or the Twitch redirect URL containing it.",
                "Введите access token Twitch или адрес перенаправления Twitch с токеном."
            )
        )
    };
    let input = input.trim();
    let token = if input.starts_with("https://") {
        let url = reqwest::Url::parse(input).map_err(|_| invalid())?;
        ensure!(
            url.host_str() == Some("www.twitch.tv")
                && url.path() == "/"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none(),
            "{}",
            invalid()
        );
        let fragment = url.fragment().ok_or_else(invalid)?;
        let mut tokens = reqwest::Url::parse(&format!("https://www.twitch.tv/?{fragment}"))
            .map_err(|_| invalid())?
            .query_pairs()
            .filter(|(name, _)| name == "access_token")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        ensure!(tokens.len() == 1, "{}", invalid());
        tokens.remove(0)
    } else {
        input
            .strip_prefix("oauth:")
            .or_else(|| input.strip_prefix("OAuth "))
            .unwrap_or(input)
            .to_owned()
    };
    ensure!(
        !token.is_empty()
            && token.len() <= 4096
            && token.chars().all(|c| c.is_ascii_alphanumeric()),
        "{}",
        invalid()
    );
    Ok(token)
}

pub async fn import(client: &Client, input: &str) -> Result<(Auth, Identity)> {
    import_at(client, input, VALIDATE).await
}

async fn import_at(client: &Client, input: &str, endpoint: &str) -> Result<(Auth, Identity)> {
    let access_token = token_input(input)?;
    let identity = validate_token_at(client, &access_token, endpoint).await?;
    let auth = Auth {
        access_token,
        refresh_token: None,
        client_id: identity.client_id.clone(),
        device_id: uuid::Uuid::new_v4().simple().to_string(),
        login: Some(identity.login.clone()),
        user_id: Some(identity.user_id.clone()),
    };
    Ok((auth, identity))
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
        // A zero lifetime can still pass Twitch validation.
        // Refresh after a 401 or a positive lifetime below the threshold.
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
        .context(tr!("Session refresh", "Обновление сессии"))?;
    if matches!(
        response.status(),
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(SessionExpired.into());
    }
    crate::twitch::check_status(
        response.status(),
        tr!("Session refresh", "Обновление сессии"),
    )?;
    let tokens: Tokens = response.json().await.context(tr!(
        "Invalid session refresh response",
        "Неверный ответ обновления сессии"
    ))?;
    auth.access_token = tokens.access_token;
    if tokens.refresh_token.is_some() {
        auth.refresh_token = tokens.refresh_token;
    }
    save(path, auth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn token_input_accepts_redirects_and_does_not_expose_invalid_secrets() {
        for input in [
            "test123",
            " oauth:test123\n",
            "OAuth test123",
            "https://www.twitch.tv/#access_token=test123&scope=user_read&token_type=bearer",
        ] {
            assert_eq!(token_input(input).unwrap(), "test123");
        }
        for input in [
            "",
            "secretwith spaces",
            "https://other.test/#access_token=private123",
            "https://www.twitch.tv/#access_token=private123&access_token=second",
            "https://www.twitch.tv/#error=private123",
            "https://www.twitch.tv/login#access_token=private123",
        ] {
            let error = token_input(input).unwrap_err().to_string();
            assert!(!error.contains("private123") && !error.contains("secretwith"));
        }
    }

    #[tokio::test]
    async fn imported_tokens_use_the_validated_client_and_have_no_refresh_token() {
        for expires in [0, 14400] {
            let (endpoint, server) = mock_oauth(vec![(
                "/validate",
                200,
                json!({
                    "client_id": "native-client", "login": "viewer", "user_id": "123", "expires_in": expires
                }),
            )]);
            let (auth, identity) = import_at(
                &Client::new(),
                "oauth:test123",
                &format!("{endpoint}/validate"),
            )
            .await
            .unwrap();
            assert_eq!(identity.expires_in, expires);
            assert_eq!(auth.client_id, "native-client");
            assert_eq!(auth.login.as_deref(), Some("viewer"));
            assert_eq!(auth.user_id.as_deref(), Some("123"));
            assert!(auth.refresh_token.is_none());
            assert_eq!(server.join().unwrap(), ["/validate"]);
        }
    }
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
                // Windows inherits the listener's nonblocking mode.
                socket.set_nonblocking(false).unwrap();
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
            login: None,
            user_id: None,
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
    fn session_is_saved_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.auth.json");
        let auth = Auth {
            access_token: "test-only".into(),
            refresh_token: None,
            client_id: "client".into(),
            device_id: "device".into(),
            login: None,
            user_id: None,
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
