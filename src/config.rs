use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(clap::Args, Default)]
pub struct SettingsPatch {
    #[arg(long)]
    pub max_channels: Option<usize>,
    #[arg(long)]
    pub poll_interval_seconds: Option<u64>,
    #[arg(long)]
    pub watch_interval_seconds: Option<u64>,
    #[arg(long, value_parser=clap::value_parser!(bool))]
    pub claim_bonus: Option<bool>,
    #[arg(long)]
    pub client_id: Option<String>,
    #[arg(
        long,
        help = "Scopes через пробел или запятую; пустая строка очищает список"
    )]
    pub oauth_scopes: Option<String>,
}
impl SettingsPatch {
    pub fn apply(self, config: &Config) -> Result<Config> {
        let mut next = config.clone();
        if let Some(value) = self.max_channels {
            next.max_channels = value;
        }
        if let Some(value) = self.poll_interval_seconds {
            next.poll_interval_seconds = value;
        }
        if let Some(value) = self.watch_interval_seconds {
            next.watch_interval_seconds = value;
        }
        if let Some(value) = self.claim_bonus {
            next.claim_bonus = value;
        }
        if let Some(value) = self.client_id {
            next.client_id = value.trim().into();
        }
        if let Some(value) = self.oauth_scopes {
            next.oauth_scopes = value
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
        next.validate()?;
        Ok(next)
    }
}

pub const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub channels: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub account_channels: BTreeMap<String, Vec<String>>,
    pub enabled_accounts: Option<Vec<String>>,
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
            account_channels: BTreeMap::new(),
            enabled_accounts: None,
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
        let mut per_account = BTreeMap::new();
        for (name, channels) in std::mem::take(&mut config.account_channels) {
            let name = crate::accounts::normalize_name(&name)?;
            let channels = normalize_channels(&channels)?;
            ensure!(
                !per_account.contains_key(&name),
                "Повторяющийся логин аккаунта: {name}"
            );
            per_account.insert(name, channels);
        }
        // An empty own list means inherit the shared list.
        per_account.retain(|_, channels| !channels.is_empty());
        config.account_channels = per_account;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let config = self;
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
        Ok(())
    }

    pub fn channels_for(&self, name: &str) -> &[String] {
        self.account_channels
            .get(name)
            .map_or(&self.channels, Vec::as_slice)
    }

    pub fn override_for_run(&mut self, values: &[String]) -> Result<()> {
        if !values.is_empty() {
            let channels = normalize_channels(values)?;
            ensure!(!channels.is_empty(), "Список каналов пуст");
            self.channels = channels;
            self.account_channels.clear();
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file =
            tempfile::NamedTempFile::new_in(parent).context("Не удалось создать файл настроек")?;
        let text = format!(
            "# Каналы перечислены в порядке приоритета.\n{}",
            toml::to_string_pretty(self)?
        );
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)
            .map_err(|_| anyhow::anyhow!("Не удалось сохранить настройки"))?;
        Ok(())
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
    fn patches_validate_settings_and_allow_clearing_scopes() {
        let config = Config::default();
        let next = SettingsPatch {
            max_channels: Some(1),
            claim_bonus: Some(false),
            oauth_scopes: Some("".into()),
            ..SettingsPatch::default()
        }
        .apply(&config)
        .unwrap();
        assert_eq!(next.max_channels, 1);
        assert!(!next.claim_bonus);
        assert!(next.oauth_scopes.is_empty());
        assert!(
            SettingsPatch {
                watch_interval_seconds: Some(1),
                ..SettingsPatch::default()
            }
            .apply(&config)
            .is_err()
        );
        assert_eq!(config.max_channels, 2);
    }

    #[test]
    fn own_lists_inherit_and_run_override_does_not_change_saved_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config {
            channels: vec!["common".into()],
            ..Config::default()
        };
        config
            .account_channels
            .insert("alt".into(), vec!["own".into()]);
        config.save(&path).unwrap();
        let mut loaded = Config::read(&path).unwrap();
        assert_eq!(loaded.channels_for("default"), ["common"]);
        assert_eq!(loaded.channels_for("alt"), ["own"]);
        loaded.override_for_run(&["temporary".into()]).unwrap();
        assert_eq!(loaded.channels_for("alt"), ["temporary"]);
        assert_eq!(Config::read(&path).unwrap().channels_for("alt"), ["own"]);
    }

    #[test]
    fn config_normalizes_profile_names_and_rejects_ambiguous_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "channels = ['common']\n[account_channels]\nALT = ['Own, @OTHER']\n",
        )
        .unwrap();
        let config = Config::read(&path).unwrap();
        assert_eq!(config.channels_for("alt"), ["own", "other"]);
        fs::write(&path, "[account_channels]\nALT = ['one']\nalt = ['two']\n").unwrap();
        assert!(Config::read(&path).is_err());
    }

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
