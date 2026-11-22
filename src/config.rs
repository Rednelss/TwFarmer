use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum FarmingPriority {
    #[default]
    Points,
    Drops,
}

impl FarmingPriority {
    pub fn code(self) -> &'static str {
        match self {
            Self::Points => "points",
            Self::Drops => "drops",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Points => tr!("channel points", "баллы каналов"),
            Self::Drops => "Drops",
        }
    }
}

#[derive(clap::Args, Default)]
pub struct SettingsPatch {
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Install stable GitHub updates automatically and restart", "Автоматически устанавливать стабильные обновления GitHub и перезапускаться"))]
    pub auto_update: Option<bool>,
    #[arg(long, value_enum, help = tr!("Fill video slots with points or Drops first", "В первую очередь занимать видеослоты баллами или Drops"))]
    pub farming_priority: Option<FarmingPriority>,
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Find eligible Drops campaigns and channels automatically", "Автоматически искать доступные кампании Drops и каналы"))]
    pub auto_drops: Option<bool>,
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Farm watch-based Drops on configured channels", "Фармить Drops за просмотр настроенных каналов"))]
    pub farm_drops: Option<bool>,
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Claim Drops when Twitch reports they are ready", "Забирать Drops, когда Twitch сообщает о готовности"))]
    pub claim_drops: Option<bool>,
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Join Twitch chat for the configured channels", "Присутствовать в чате настроенных каналов Twitch"))]
    pub chat_presence: Option<bool>,
    #[arg(long, value_enum, help = tr!("Saved interface and log language: en or ru", "Сохранённый язык интерфейса и журнала: en или ru"))]
    pub language: Option<crate::i18n::Language>,
    #[arg(long, help = tr!("Total video streams per account (1-2), including at most one Drops stream", "Всего видеопотоков на аккаунт (1-2), в том числе не более одного для Drops"))]
    pub max_channels: Option<usize>,
    #[arg(long, help = tr!("Balance polling interval in seconds (30-3600)", "Интервал опроса балансов в секундах (30-3600)"))]
    pub poll_interval_seconds: Option<u64>,
    #[arg(long, help = tr!("Watch request interval in seconds (15-60)", "Интервал запросов просмотра в секундах (15-60)"))]
    pub watch_interval_seconds: Option<u64>,
    #[arg(long, value_parser=clap::value_parser!(bool), help = tr!("Claim available channel point bonuses automatically", "Автоматически забирать доступные бонусы баллов канала"))]
    pub claim_bonus: Option<bool>,
}
impl SettingsPatch {
    pub fn apply(self, config: &Config) -> Result<Config> {
        let mut next = config.clone();
        if let Some(value) = self.auto_update {
            next.auto_update = value;
        }
        if let Some(value) = self.farming_priority {
            next.farming_priority = value;
        }
        if let Some(value) = self.auto_drops {
            next.auto_drops = value;
        }
        if let Some(value) = self.farm_drops {
            next.farm_drops = value;
        }
        if let Some(value) = self.claim_drops {
            next.claim_drops = value;
        }
        if let Some(value) = self.chat_presence {
            next.chat_presence = value;
        }
        if let Some(language) = self.language {
            next.language = language;
        }
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
        next.validate()?;
        Ok(next)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub auto_update: bool,
    pub farming_priority: FarmingPriority,
    pub auto_drops: bool,
    pub farm_drops: bool,
    pub claim_drops: bool,
    pub chat_presence: bool,
    pub language: crate::i18n::Language,
    pub channels: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub account_channels: BTreeMap<String, Vec<String>>,
    pub enabled_accounts: Option<Vec<String>>,
    pub max_channels: usize,
    pub poll_interval_seconds: u64,
    pub watch_interval_seconds: u64,
    pub claim_bonus: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_update: true,
            farming_priority: FarmingPriority::Points,
            auto_drops: false,
            farm_drops: true,
            claim_drops: true,
            chat_presence: true,
            language: crate::i18n::Language::En,
            channels: vec![],
            account_channels: BTreeMap::new(),
            enabled_accounts: None,
            max_channels: 2,
            poll_interval_seconds: 60,
            watch_interval_seconds: 20,
            claim_bonus: true,
        }
    }
}

impl Config {
    pub fn automatic_drops(&self) -> bool {
        self.farm_drops && self.auto_drops
    }
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| {
            tf!(
                "Could not read {}. Run twfarmer init.",
                "Не удалось прочитать {}. Выполните twfarmer init.",
                path.display()
            )
        })?;
        let mut value: toml::Value = toml::from_str(&text).context(tr!(
            "Invalid TOML configuration",
            "Ошибка в TOML-конфигурации"
        ))?;
        if let Some(table) = value.as_table_mut() {
            table.remove("client_id");
            table.remove("oauth_scopes");
        }
        let mut config: Self = value.try_into().context(tr!(
            "Invalid TOML configuration",
            "Ошибка в TOML-конфигурации"
        ))?;
        config.channels = normalize_channels(&config.channels)?;
        let mut per_account = BTreeMap::new();
        for (name, channels) in std::mem::take(&mut config.account_channels) {
            let name = crate::accounts::normalize_name(&name)?;
            let channels = normalize_channels(&channels)?;
            ensure!(
                !per_account.contains_key(&name),
                "{}",
                tf!(
                    "Duplicate account login: {name}",
                    "Повторяющийся логин аккаунта: {name}"
                )
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
            "{}",
            tf!(
                "max_channels must be 1 or 2",
                "max_channels должен быть 1 или 2"
            )
        );
        ensure!(
            (30..=3600).contains(&config.poll_interval_seconds),
            "{}",
            tf!(
                "poll_interval_seconds: from 30 to 3600",
                "poll_interval_seconds: от 30 до 3600"
            )
        );
        ensure!(
            (15..=60).contains(&config.watch_interval_seconds),
            "{}",
            tf!(
                "watch_interval_seconds: from 15 to 60",
                "watch_interval_seconds: от 15 до 60"
            )
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
            ensure!(
                !channels.is_empty(),
                "{}",
                tf!("The channel list is empty", "Список каналов пуст")
            );
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
        let mut file = tempfile::NamedTempFile::new_in(parent).context(tr!(
            "Could not create the settings file",
            "Не удалось создать файл настроек"
        ))?;
        let text = tf!(
            "# Channels are listed in priority order.\n{}",
            "# Каналы перечислены в порядке приоритета.\n{}",
            toml::to_string_pretty(self)?
        );
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|_| {
            anyhow::anyhow!(
                "{}",
                tf!("Could not save settings", "Не удалось сохранить настройки")
            )
        })?;
        Ok(())
    }

    pub fn create(path: &Path, channels: &[String]) -> Result<()> {
        ensure!(
            !path.exists(),
            "{}",
            tf!(
                "{} already exists; edit the file instead.",
                "{} уже существует; отредактируйте файл.",
                path.display()
            )
        );
        let config = Self {
            channels: normalize_channels(channels)?,
            ..Self::default()
        };

        let text = tf!(
            "# Channels are listed in priority order.\n{}",
            "# Каналы перечислены в порядке приоритета.\n{}",
            toml::to_string_pretty(&config)?
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                tf!(
                    "Could not create {}",
                    "Не удалось создать {}",
                    path.display()
                )
            })?;
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
                let url = reqwest::Url::parse(input)
                    .context(tr!("Invalid channel URL", "Неверная ссылка на канал"))?;
                ensure!(
                    matches!(
                        url.host_str(),
                        Some("twitch.tv" | "www.twitch.tv" | "m.twitch.tv")
                    ),
                    "{}",
                    tf!("A Twitch URL was expected", "Ожидалась ссылка Twitch")
                );
                let mut segments = url
                    .path_segments()
                    .context(tr!(
                        "The URL does not contain a channel",
                        "В ссылке нет канала"
                    ))?
                    .filter(|s| !s.is_empty());
                let login = segments
                    .next()
                    .context(tr!(
                        "The URL does not contain a channel",
                        "В ссылке нет канала"
                    ))?
                    .to_string();
                ensure!(
                    segments.next().is_none(),
                    "{}",
                    tf!(
                        "Specify the channel's main page",
                        "Укажите главную страницу канала"
                    )
                );
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
                "{}",
                tf!(
                    "Invalid channel name: {input}",
                    "Неверное имя канала: {input}"
                )
            );
            if seen.insert(login.clone()) {
                channels.push(login);
            }
        }
    }
    if channels.len() > 100 {
        bail!(
            "{}",
            tf!(
                "At most 100 channels are allowed",
                "Допускается не более 100 каналов"
            )
        );
    }
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patches_validate_settings() {
        let config = Config::default();
        let next = SettingsPatch {
            max_channels: Some(1),
            claim_bonus: Some(false),
            ..SettingsPatch::default()
        }
        .apply(&config)
        .unwrap();
        assert_eq!(next.max_channels, 1);
        assert!(!next.claim_bonus);
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
    fn legacy_authorization_settings_do_not_affect_saved_farming_options() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "channels = ['channel']\nclient_id = 'old-tv-client'\noauth_scopes = ['user_read']\nmax_channels = 1\n").unwrap();
        let config = Config::read(&path).unwrap();
        assert_eq!(config.channels, ["channel"]);
        assert_eq!(config.max_channels, 1);
        config.save(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("client_id") && !saved.contains("oauth_scopes"));
        fs::write(&path, "unknown_option = true\n").unwrap();
        assert!(Config::read(&path).is_err());
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
