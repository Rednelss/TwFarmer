use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub channels: Vec<String>,
    pub max_channels: usize,
    pub poll_interval_seconds: u64,
    pub watch_interval_seconds: u64,
    pub claim_bonus: bool,
    pub client_id: String,
    pub oauth_scopes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channels: vec![],
            max_channels: 2,
            poll_interval_seconds: 60,
            watch_interval_seconds: 20,
            claim_bonus: true,
            client_id: CLIENT_ID.into(),
            oauth_scopes: vec!["user_read".into()],
        }
    }
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| {
            format!(
                "Не удалось прочитать {}. Выполните twfarmer init.",
                path.display()
            )
        })?;
        let mut config: Self = toml::from_str(&text).context("Ошибка в TOML-конфигурации")?;
        config.channels = normalize_channels(&config.channels)?;
        ensure!(
            (1..=2).contains(&config.max_channels),
            "max_channels должен быть 1 или 2"
        );
        ensure!(
            (30..=3600).contains(&config.poll_interval_seconds),
            "poll_interval_seconds: от 30 до 3600"
        );
        ensure!(
            (15..=60).contains(&config.watch_interval_seconds),
            "watch_interval_seconds: от 15 до 60"
        );
        ensure!(
            !config.client_id.is_empty()
                && config.client_id.chars().all(|c| c.is_ascii_alphanumeric()),
            "Неверный client_id"
        );
        Ok(config)
    }

    pub fn create(path: &Path, channels: &[String]) -> Result<()> {
        ensure!(
            !path.exists(),
            "{} уже существует; отредактируйте файл.",
            path.display()
        );
        let config = Self {
            channels: normalize_channels(channels)?,
            ..Self::default()
        };
        ensure!(!config.channels.is_empty(), "Укажите хотя бы один канал");
        let text = format!(
            "# Каналы перечислены в порядке приоритета.\n{}",
            toml::to_string_pretty(&config)?
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("Не удалось создать {}", path.display()))?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }
}

pub fn auth_path(config: &Path) -> PathBuf {
    let mut name = config.as_os_str().to_os_string();
    name.push(".auth.json");
    PathBuf::from(name)
}

pub fn normalize_channels(values: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut channels = vec![];
    for value in values {
        for input in value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
        {
            let login = if input.starts_with("https://") || input.starts_with("http://") {
                let url = reqwest::Url::parse(input).context("Неверная ссылка на канал")?;
                ensure!(
                    matches!(
                        url.host_str(),
                        Some("twitch.tv" | "www.twitch.tv" | "m.twitch.tv")
                    ),
                    "Ожидалась ссылка Twitch"
                );
                let mut segments = url
                    .path_segments()
                    .context("В ссылке нет канала")?
                    .filter(|s| !s.is_empty());
                let login = segments.next().context("В ссылке нет канала")?.to_string();
                ensure!(segments.next().is_none(), "Укажите главную страницу канала");
                login
            } else {
                input
                    .trim_start_matches('@')
                    .trim_end_matches('/')
                    .to_string()
            }
            .to_ascii_lowercase();
            ensure!(
                !login.is_empty()
                    && login.len() <= 25
                    && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Неверное имя канала: {input}"
            );
            if seen.insert(login.clone()) {
                channels.push(login);
            }
        }
    }
    if channels.len() > 100 {
        bail!("Допускается не более 100 каналов");
    }
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_commas_and_duplicates() {
        assert_eq!(
            normalize_channels(&["Foo, @BAR https://www.twitch.tv/foo/".into()]).unwrap(),
            ["foo", "bar"]
        );
    }
    #[test]
    fn rejects_other_hosts_paths_and_empty_url() {
        for input in [
            "https://example.org/foo",
            "https://twitch.tv/foo/videos",
            "https://twitch.tv/",
            "foo<script>",
        ] {
            assert!(normalize_channels(&[input.into()]).is_err(), "{input}");
        }
    }
}
