use crate::model::{ChatStatus, Reporter};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::watch,
    time::{Instant, MissedTickBehavior, interval, sleep, timeout},
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

const HOST: &str = "irc.chat.twitch.tv";
const MAX_LINE: usize = 64 * 1024;
const JOIN_INTERVAL: Duration = Duration::from_millis(600);

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    login: String,
    token: String,
}

impl Credentials {
    pub fn new(login: String, token: String) -> Self {
        Self { login, token }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            valid_name(&self.login) && !self.token.is_empty() && !self.token.contains(['\r', '\n']),
            "{}",
            tr!("Invalid chat credentials", "Неверные данные входа в чат")
        );
        Ok(())
    }
}

#[derive(Debug)]
struct AuthenticationRejected;

impl std::fmt::Display for AuthenticationRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(tr!(
            "Twitch rejected chat login. Renew the account login; IRC tokens may need chat:read permission.",
            "Twitch отклонил вход в чат. Обновите вход в аккаунт; токену IRC может требоваться право chat:read."
        ))
    }
}

impl std::error::Error for AuthenticationRejected {}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

struct Message<'a> {
    prefix: Option<&'a str>,
    command: &'a str,
    parameters: &'a str,
}

fn parse(line: &str) -> Option<Message<'_>> {
    let mut line = line.trim_end_matches(['\r', '\n']);
    if line.starts_with('@') {
        line = line.split_once(' ')?.1;
    }
    let prefix = if let Some(rest) = line.strip_prefix(':') {
        let (prefix, rest) = rest.split_once(' ')?;
        line = rest;
        Some(prefix)
    } else {
        None
    };
    let (command, parameters) = line.split_once(' ').unwrap_or((line, ""));
    (!command.is_empty()).then_some(Message {
        prefix,
        command,
        parameters,
    })
}

async fn read_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    bytes: &mut Vec<u8>,
) -> Result<()> {
    let remaining = MAX_LINE.saturating_sub(bytes.len()) + 1;
    let count = (&mut *reader)
        .take(remaining as u64)
        .read_until(b'\n', bytes)
        .await
        .context(tr!("Reading Twitch chat", "Чтение чата Twitch"))?;
    ensure!(
        count > 0,
        "{}",
        tr!(
            "Twitch closed the chat connection",
            "Twitch закрыл соединение с чатом"
        )
    );
    ensure!(
        bytes.len() <= MAX_LINE && bytes.ends_with(b"\n"),
        "{}",
        tr!(
            "Invalid or oversized IRC message",
            "Неверное или слишком длинное сообщение IRC"
        )
    );
    Ok(())
}

async fn send<W: AsyncWrite + Unpin>(writer: &mut W, command: &str) -> Result<()> {
    timeout(
        Duration::from_secs(10),
        writer.write_all(command.as_bytes()),
    )
    .await
    .context(tr!(
        "Chat write timed out",
        "Истекло время отправки команды чата"
    ))?
    .context(tr!("Sending a chat command", "Отправка команды чата"))
}

async fn connect() -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = tokio_rustls::rustls::crypto::ring::default_provider();
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .context(tr!("Configuring chat TLS", "Настройка TLS для чата"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let socket = timeout(Duration::from_secs(15), TcpStream::connect((HOST, 6697)))
        .await
        .context(tr!(
            "Chat connection timed out",
            "Истекло время подключения к чату"
        ))?
        .context(tr!(
            "Connecting to Twitch chat",
            "Подключение к чату Twitch"
        ))?;
    timeout(
        Duration::from_secs(15),
        TlsConnector::from(Arc::new(config)).connect(ServerName::try_from(HOST)?, socket),
    )
    .await
    .context(tr!(
        "Chat TLS handshake timed out",
        "Истекло время TLS-подключения к чату"
    ))?
    .context(tr!(
        "Securing the Twitch chat connection",
        "Защищённое подключение к чату Twitch"
    ))
}

async fn session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    credentials: &Credentials,
    channels: &[String],
    reporter: &Reporter,
    changes: &mut watch::Receiver<Credentials>,
) -> Result<()> {
    credentials.validate()?;
    ensure!(
        channels.iter().all(|channel| valid_name(channel)),
        "{}",
        tr!("Invalid chat channel name", "Неверное имя канала чата")
    );
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    send(
        &mut writer,
        &format!(
            "PASS oauth:{}\r\nNICK {}\r\nCAP REQ :twitch.tv/membership twitch.tv/commands\r\n",
            credentials.token, credentials.login
        ),
    )
    .await?;
    let mut bytes = Vec::new();
    let mut waiting: VecDeque<_> = channels.iter().cloned().collect();
    let wanted: HashSet<_> = channels.iter().cloned().collect();
    let mut pending = HashMap::new();
    let mut joined = HashSet::new();
    let mut authenticated = false;
    let mut last_received = Instant::now();
    let login_deadline = Instant::now() + Duration::from_secs(20);
    // Stay below Twitch's limit of 20 JOIN requests per 10 seconds.
    let mut joins = interval(JOIN_INTERVAL);
    let mut heartbeat = interval(Duration::from_secs(60));
    let mut watchdog = interval(Duration::from_secs(1));
    joins.set_missed_tick_behavior(MissedTickBehavior::Skip);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = changes.changed() => {
                result.context(tr!("Chat credentials are no longer available", "Данные входа в чат больше недоступны"))?;
                return Ok(());
            }
            result = read_line(&mut reader, &mut bytes) => {
                result?;
                last_received = Instant::now();
                let line = std::str::from_utf8(&bytes).context(tr!("Invalid chat encoding", "Неверная кодировка чата"))?;
                let Some(message) = parse(line) else { bytes.clear(); continue; };
                match message.command {
                    "PING" => send(&mut writer, &format!("PONG {}\r\n", message.parameters)).await?,
                    "001" => authenticated = true,
                    "464" => return Err(AuthenticationRejected.into()),
                    "RECONNECT" => bail!("{}", tr!("Twitch requested a chat reconnect", "Twitch запросил переподключение чата")),
                    "NOTICE" if ["authentication failed", "improperly formatted auth", "login unsuccessful", "invalid oauth"].iter().any(|text| message.parameters.to_ascii_lowercase().contains(text)) => return Err(AuthenticationRejected.into()),
                    "NOTICE" => {
                        if let Some(channel) = message.parameters.split_whitespace().find_map(|word| word.strip_prefix('#'))
                            && pending.remove(channel).is_some()
                        {
                            let error = tr!("Twitch rejected chat membership for this channel.", "Twitch отклонил подключение к чату этого канала.").to_string();
                            reporter.chat(channel, ChatStatus::Error, Some(error.clone()));
                            reporter.log(format!("{channel}: {error}"), true);
                        }
                    }
                    "JOIN" | "PART" => {
                        if message.prefix.and_then(|prefix| prefix.split('!').next()) != Some(credentials.login.as_str()) { bytes.clear(); continue; }
                        let channel = message.parameters.trim_start_matches(':').trim_start_matches('#').trim();
                        if !wanted.contains(channel) { bytes.clear(); continue; }
                        if message.command == "JOIN" {
                            pending.remove(channel);
                            if joined.insert(channel.to_owned()) {
                                reporter.chat(channel, ChatStatus::Joined, None);
                                reporter.log(tf!("{channel}: joined Twitch chat.", "{channel}: подключён к чату Twitch."), false);
                            }
                        } else {
                            bail!("{}", tr!("Twitch ended chat membership", "Twitch отключил аккаунт от чата"));
                        }
                    }
                    "403" | "405" | "471" | "473" | "474" | "475" | "476" | "477" => {
                        if let Some(channel) = message.parameters.split_whitespace().find_map(|word| word.strip_prefix('#'))
                            && wanted.contains(channel)
                        {
                            pending.remove(channel);
                            let error = tf!("Chat join rejected (IRC {}).", "Подключение к чату отклонено (IRC {}).", message.command);
                            reporter.chat(channel, ChatStatus::Error, Some(error.clone()));
                            reporter.log(format!("{channel}: {error}"), true);
                        }
                    }
                    _ => {}
                }
                bytes.clear();
            }
            _ = joins.tick(), if authenticated && !waiting.is_empty() => {
                let channel = waiting.pop_front().expect("join queue is nonempty");
                send(&mut writer, &format!("JOIN #{channel}\r\n")).await?;
                pending.insert(channel, Instant::now());
            }
            _ = heartbeat.tick(), if authenticated => send(&mut writer, "PING :twfarmer\r\n").await?,
            _ = watchdog.tick() => {
                ensure!(authenticated || Instant::now() < login_deadline, "{}", tr!("Twitch did not confirm chat login", "Twitch не подтвердил вход в чат"));
                ensure!(last_received.elapsed() < Duration::from_secs(150), "{}", tr!("Twitch chat stopped responding", "Чат Twitch перестал отвечать"));
                ensure!(!pending.values().any(|sent: &Instant| sent.elapsed() >= Duration::from_secs(20)), "{}", tr!("Twitch did not confirm a chat join", "Twitch не подтвердил подключение к чату"));
            }
        }
    }
}

pub async fn run(
    channels: Vec<String>,
    credentials: watch::Receiver<Credentials>,
    reporter: Reporter,
) {
    run_with(channels, credentials, reporter, connect).await;
}

async fn run_with<S, F, Fut>(
    channels: Vec<String>,
    mut credentials: watch::Receiver<Credentials>,
    reporter: Reporter,
    mut open: F,
) where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S>>,
{
    let mut failures = 0u32;
    loop {
        for channel in &channels {
            reporter.chat(channel, ChatStatus::Connecting, None);
        }
        let current = credentials.borrow_and_update().clone();
        let started = Instant::now();
        let result = async {
            current.validate()?;
            let stream = open().await?;
            session(stream, &current, &channels, &reporter, &mut credentials).await
        }
        .await;
        match result {
            Ok(()) => {
                if credentials.has_changed().is_err() {
                    return;
                }
                failures = 0;
            }
            Err(error) => {
                if credentials.has_changed().is_err() {
                    return;
                }
                if error.is::<AuthenticationRejected>() {
                    let message = format!("{error:#}");
                    for channel in &channels {
                        reporter.chat(channel, ChatStatus::Error, Some(message.clone()));
                    }
                    reporter.log(message, true);
                    if credentials.changed().await.is_err() {
                        return;
                    }
                    failures = 0;
                    continue;
                }
                if started.elapsed() >= Duration::from_secs(300) {
                    failures = 0;
                }
                failures = failures.saturating_add(1);
                let delay =
                    (20u64.saturating_mul(1u64 << failures.min(5).saturating_sub(1))).min(300);
                let retry = Instant::now() + Duration::from_secs(delay);
                let message = format!("{error:#}");
                for channel in &channels {
                    reporter.chat(channel, ChatStatus::Retry(retry), Some(message.clone()));
                }
                reporter.log(
                    tf!(
                        "Chat: {message}. Retrying in {delay}s.",
                        "Чат: {message}. Повтор через {delay}с."
                    ),
                    true,
                );
                tokio::select! {
                    _ = sleep(Duration::from_secs(delay)) => {}
                    result = credentials.changed() => if result.is_err() { return; } else { failures = 0; }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Event;
    use futures_util::FutureExt;
    use tokio::io::DuplexStream;
    use tokio::sync::mpsc;

    async fn command(reader: &mut BufReader<tokio::io::ReadHalf<DuplexStream>>) -> String {
        let mut line = String::new();
        timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        assert!(!line.is_empty());
        line
    }

    async fn greeting(reader: &mut BufReader<tokio::io::ReadHalf<DuplexStream>>, token: &str) {
        assert_eq!(command(reader).await, format!("PASS oauth:{token}\r\n"));
        assert_eq!(command(reader).await, "NICK test_user\r\n");
        assert_eq!(
            command(reader).await,
            "CAP REQ :twitch.tv/membership twitch.tv/commands\r\n"
        );
    }

    #[tokio::test]
    async fn partial_lines_survive_read_cancellation_and_are_bounded() {
        let (client, mut server) = tokio::io::duplex(MAX_LINE * 2);
        let mut reader = BufReader::new(client);
        let mut bytes = Vec::new();
        server.write_all(b"PING :part").await.unwrap();
        assert!(read_line(&mut reader, &mut bytes).now_or_never().is_none());
        assert_eq!(bytes, b"PING :part");
        server.write_all(b"ial\r\n").await.unwrap();
        read_line(&mut reader, &mut bytes).await.unwrap();
        assert_eq!(bytes, b"PING :partial\r\n");
        bytes.clear();
        server.write_all(&vec![b'x'; MAX_LINE + 1]).await.unwrap();
        assert!(read_line(&mut reader, &mut bytes).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn joins_all_channels_answers_ping_and_never_sends_chat_messages() {
        let (client, server) = tokio::io::duplex(8192);
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = BufReader::new(reader);
        let credentials = Credentials::new("test_user".into(), "private-test-token".into());
        let (updates, mut changes) = watch::channel(credentials.clone());
        let (events, mut received) = mpsc::unbounded_channel();
        let reporter = Reporter::new("account".into(), events);
        let task = tokio::spawn(async move {
            session(
                client,
                &credentials,
                &["channel_one".into(), "channel_two".into()],
                &reporter,
                &mut changes,
            )
            .await
        });
        greeting(&mut reader, "private-test-token").await;
        writer
            .write_all(b":tmi.twitch.tv 001 test_user :Welcome\r\nPING :ping-token\r\n")
            .await
            .unwrap();
        let mut rooms = HashSet::new();
        let mut pong = false;
        while rooms.len() < 2 || !pong {
            let line = command(&mut reader).await;
            if line == "PONG :ping-token\r\n" {
                pong = true;
            } else if let Some(room) = line.strip_prefix("JOIN #") {
                let room = room.trim().to_string();
                rooms.insert(room.clone());
                writer.write_all(format!(":other_user!other_user@host JOIN #{room}\r\n:other_user!other_user@host PRIVMSG #{room} :private incoming message\r\n@test=tag :test_user!test_user@host JOIN #{room}\r\n").as_bytes()).await.unwrap();
            } else if let Some(payload) = line.strip_prefix("PING ") {
                writer
                    .write_all(format!("PONG {payload}").as_bytes())
                    .await
                    .unwrap();
            } else {
                panic!("Unexpected outgoing IRC command");
            }
        }
        assert_eq!(
            rooms,
            HashSet::from(["channel_one".into(), "channel_two".into()])
        );
        let mut confirmed = HashSet::new();
        while confirmed.len() < 2 {
            match timeout(Duration::from_secs(5), received.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Chat {
                    channel,
                    status: ChatStatus::Joined,
                    ..
                } => {
                    confirmed.insert(channel);
                }
                Event::Log { message, .. } => {
                    assert!(!message.contains("private-test-token"));
                    assert!(!message.contains("private incoming message"));
                }
                _ => {}
            }
        }
        updates
            .send(Credentials::new("test_user".into(), "new-token".into()))
            .unwrap();
        assert!(
            timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_rejected_channel_does_not_disconnect_other_chats() {
        let (client, server) = tokio::io::duplex(8192);
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = BufReader::new(reader);
        let credentials = Credentials::new("test_user".into(), "token".into());
        let (updates, mut changes) = watch::channel(credentials.clone());
        let (events, mut received) = mpsc::unbounded_channel();
        let reporter = Reporter::new("account".into(), events);
        let task = tokio::spawn(async move {
            session(
                client,
                &credentials,
                &["blocked".into(), "available".into()],
                &reporter,
                &mut changes,
            )
            .await
        });
        greeting(&mut reader, "token").await;
        writer
            .write_all(b":tmi.twitch.tv 001 test_user :Welcome\r\n")
            .await
            .unwrap();
        let mut joins = 0;
        while joins < 2 {
            let line = command(&mut reader).await;
            if line == "JOIN #blocked\r\n" {
                writer
                    .write_all(b":tmi.twitch.tv NOTICE #blocked :msg_channel_suspended\r\n")
                    .await
                    .unwrap();
                joins += 1;
            } else if line == "JOIN #available\r\n" {
                writer
                    .write_all(b":test_user!test_user@host JOIN #available\r\n")
                    .await
                    .unwrap();
                joins += 1;
            } else if let Some(payload) = line.strip_prefix("PING ") {
                writer
                    .write_all(format!("PONG {payload}").as_bytes())
                    .await
                    .unwrap();
            }
        }
        let mut blocked = false;
        let mut available = false;
        while !blocked || !available {
            if let Event::Chat {
                channel, status, ..
            } = received.recv().await.unwrap()
            {
                blocked |= channel == "blocked" && status == ChatStatus::Error;
                available |= channel == "available" && status == ChatStatus::Joined;
            }
        }
        tokio::time::advance(Duration::from_secs(21)).await;
        assert!(!task.is_finished());
        updates
            .send(Credentials::new("test_user".into(), "new-token".into()))
            .unwrap();
        assert!(
            timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn authentication_rejection_waits_for_new_credentials_then_reconnects() {
        let (updates, changes) =
            watch::channel(Credentials::new("test_user".into(), "old-token".into()));
        let (events, mut received) = mpsc::unbounded_channel();
        let reporter = Reporter::new("account".into(), events);
        let (peers, mut connections) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_with(
            vec!["channel".into()],
            changes,
            reporter,
            move || {
                let (client, server) = tokio::io::duplex(8192);
                peers.send(server).unwrap();
                async { Ok(client) }
            },
        ));
        let (reader, mut writer) = tokio::io::split(connections.recv().await.unwrap());
        let mut reader = BufReader::new(reader);
        greeting(&mut reader, "old-token").await;
        writer
            .write_all(b":tmi.twitch.tv NOTICE * :Login authentication failed\r\n")
            .await
            .unwrap();
        loop {
            if let Event::Chat {
                status: ChatStatus::Error,
                error,
                ..
            } = received.recv().await.unwrap()
            {
                assert!(error.unwrap().contains("chat:read"));
                break;
            }
        }
        tokio::time::advance(Duration::from_secs(600)).await;
        assert!(matches!(
            connections.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        updates
            .send(Credentials::new("test_user".into(), "new-token".into()))
            .unwrap();
        let (reader, mut writer) = tokio::io::split(connections.recv().await.unwrap());
        let mut reader = BufReader::new(reader);
        greeting(&mut reader, "new-token").await;
        writer
            .write_all(b":tmi.twitch.tv 001 test_user :Welcome\r\n")
            .await
            .unwrap();
        loop {
            let line = command(&mut reader).await;
            if line == "JOIN #channel\r\n" {
                break;
            }
            if let Some(payload) = line.strip_prefix("PING ") {
                writer
                    .write_all(format!("PONG {payload}").as_bytes())
                    .await
                    .unwrap();
            }
        }
        writer
            .write_all(b":test_user!test_user@host JOIN #channel\r\n")
            .await
            .unwrap();
        loop {
            if matches!(
                received.recv().await.unwrap(),
                Event::Chat {
                    status: ChatStatus::Joined,
                    ..
                }
            ) {
                break;
            }
        }
        drop(reader);
        drop(writer);
        loop {
            if matches!(
                received.recv().await.unwrap(),
                Event::Chat {
                    status: ChatStatus::Retry(_),
                    ..
                }
            ) {
                break;
            }
        }
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(connections.recv().await.is_some());
        drop(updates);
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires TWFARMER_CHAT_SMOKE_CONFIG and a saved Twitch session"]
    async fn saved_session_receives_own_join_confirmation() {
        let path = std::path::PathBuf::from(
            std::env::var("TWFARMER_CHAT_SMOKE_CONFIG").expect("set TWFARMER_CHAT_SMOKE_CONFIG"),
        );
        let config = crate::config::Config::read(&path).unwrap();
        let profile = crate::accounts::discover(&path)
            .unwrap()
            .into_iter()
            .find(|profile| {
                profile.login.is_some() && !config.channels_for(&profile.config_key).is_empty()
            })
            .expect("configured account required");
        let auth = crate::auth::read(&profile.path).unwrap();
        let channel = config.channels_for(&profile.config_key)[0].clone();
        let credentials = Credentials::new(profile.login.clone().unwrap(), auth.access_token);
        let (updates, changes) = watch::channel(credentials);
        let (events, mut received) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(
            vec![channel],
            changes,
            Reporter::new(profile.name, events),
        ));
        let confirmed = timeout(Duration::from_secs(45), async {
            while let Some(event) = received.recv().await {
                if let Event::Chat { status, error, .. } = event {
                    match status {
                        ChatStatus::Joined => return Ok(()),
                        ChatStatus::Error | ChatStatus::Retry(_) => {
                            return Err(error.unwrap_or_else(|| "chat unavailable".into()));
                        }
                        _ => {}
                    }
                }
            }
            Err("chat task ended".into())
        })
        .await;
        drop(updates);
        timeout(Duration::from_secs(20), task)
            .await
            .unwrap()
            .unwrap();
        confirmed
            .expect("chat join timed out")
            .expect("chat join failed");
    }

    #[test]
    fn credentials_reject_command_injection() {
        assert!(
            Credentials::new(
                "test_user\r\nPRIVMSG #channel :message".into(),
                "token".into()
            )
            .validate()
            .is_err()
        );
        assert!(
            Credentials::new("test_user".into(), "token\r\nJOIN #another".into())
                .validate()
                .is_err()
        );
        assert!(
            Credentials::new("test_user".into(), "token".into())
                .validate()
                .is_ok()
        );
    }
}
