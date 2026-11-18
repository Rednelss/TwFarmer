use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

#[derive(Serialize, Deserialize)]
pub enum Request {
    Start {
        accounts: Vec<String>,
        channels: Vec<String>,
    },
    Stop,
    Quit,
    Reload,
    ReloadSelection,
    View {
        journal: bool,
    },
}
#[derive(Serialize, Deserialize)]
struct Endpoint {
    address: String,
    token: String,
}
#[derive(Serialize, Deserialize)]
struct Message {
    token: String,
    request: Request,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    ok: bool,
    message: String,
}

pub fn key(config: &Path) -> Result<PathBuf> {
    if config.exists() {
        return config
            .canonicalize()
            .context("Не удалось определить путь настроек");
    }
    let parent = config
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent
        .canonicalize()?
        .join(config.file_name().context("Неверный путь настроек")?))
}
fn sidecar(key: &Path, suffix: &str) -> PathBuf {
    let mut value = key.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub struct Server {
    pub key: PathBuf,
    listener: TcpListener,
    token: String,
    endpoint_path: PathBuf,
    _lock: File,
}
impl Server {
    pub async fn bind(config: &Path) -> Result<Self> {
        let key = key(config)?;
        let lock_path = sidecar(&key, ".control.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .context("Не удалось открыть блокировку программы")?;
        lock.try_lock().context("Для этой конфигурации программа уже запущена. Используйте start, stop, reload или меню.")?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let endpoint = Endpoint {
            address: listener.local_addr()?.to_string(),
            token: token.clone(),
        };
        let endpoint_path = sidecar(&key, ".control.json");
        let mut file = tempfile::NamedTempFile::new_in(key.parent().unwrap())?;
        file.write_all(&serde_json::to_vec(&endpoint)?)?;
        file.persist(&endpoint_path)
            .context("Не удалось сохранить адрес управления")?;
        Ok(Self {
            key,
            listener,
            token,
            endpoint_path,
            _lock: lock,
        })
    }
    pub async fn accept(&self) -> Result<Connection> {
        let (stream, _) = self.listener.accept().await?;
        Ok(Connection {
            stream,
            token: self.token.clone(),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.endpoint_path);
    }
}
pub struct Connection {
    stream: TcpStream,
    token: String,
}
impl Connection {
    pub async fn request(&mut self) -> Result<Request> {
        let mut bytes = Vec::new();
        timeout(
            Duration::from_secs(2),
            (&mut self.stream).take(16385).read_to_end(&mut bytes),
        )
        .await??;
        ensure!(bytes.len() <= 16384, "Слишком длинная команда управления");
        let message: Message =
            serde_json::from_slice(&bytes).context("Неверная команда управления")?;
        ensure!(
            message.token == self.token,
            "Нет доступа к управлению экземпляром"
        );
        Ok(message.request)
    }
    pub async fn reply(mut self, result: Result<String>) -> Result<()> {
        let reply = match result {
            Ok(message) => Reply { ok: true, message },
            Err(error) => Reply {
                ok: false,
                message: format!("{error:#}"),
            },
        };
        timeout(
            Duration::from_secs(2),
            self.stream.write_all(&serde_json::to_vec(&reply)?),
        )
        .await??;
        self.stream.shutdown().await?;
        Ok(())
    }
}
pub async fn send(config: &Path, request: Request) -> Result<String> {
    let endpoint:Endpoint=serde_json::from_slice(&fs::read(sidecar(&key(config)?,".control.json"))
        .context("Нет запущенной программы для этой конфигурации. Сначала откройте интерфейс или выполните run.")?)?;
    let address: std::net::SocketAddr = endpoint.address.parse()?;
    ensure!(
        address.ip().is_loopback(),
        "Неверный адрес локального управления"
    );
    let mut stream = timeout(Duration::from_secs(2), TcpStream::connect(address))
        .await?
        .context("Запущенная программа недоступна. Запустите её заново.")?;
    stream
        .write_all(&serde_json::to_vec(&Message {
            token: endpoint.token,
            request,
        })?)
        .await?;
    stream.shutdown().await?;
    let mut bytes = Vec::new();
    timeout(
        Duration::from_secs(5),
        (&mut stream).take(16385).read_to_end(&mut bytes),
    )
    .await??;
    ensure!(bytes.len() <= 16384, "Слишком длинный ответ управления");
    let reply: Reply = serde_json::from_slice(&bytes).context("Нет ответа программы")?;
    ensure!(reply.ok, "{}", reply.message);
    Ok(reply.message)
}

pub async fn notify_reload(config: &Path) -> Result<()> {
    if sidecar(&key(config)?, ".control.json").is_file() {
        send(config, Request::Reload).await?;
    }
    Ok(())
}

pub async fn notify_selection(config: &Path) -> Result<()> {
    if sidecar(&key(config)?, ".control.json").is_file() {
        send(config, Request::ReloadSelection).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn instance_lock_and_local_commands_are_bound_to_the_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let server = Server::bind(&config).await.unwrap();
        assert!(Server::bind(&config).await.is_err());
        let path = config.clone();
        let sender = tokio::spawn(async move { send(&path, Request::Stop).await });
        let mut connection = server.accept().await.unwrap();
        assert!(matches!(connection.request().await.unwrap(), Request::Stop));
        connection.reply(Ok("stopped".into())).await.unwrap();
        assert_eq!(sender.await.unwrap().unwrap(), "stopped");
        drop(server);
        assert!(!sidecar(&config, ".control.json").exists());
        Server::bind(&config).await.unwrap();
    }
}
