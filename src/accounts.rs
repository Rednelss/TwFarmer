use crate::{
    auth::{self, Auth, Identity},
    config::{self, Config},
    twitch,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub path: PathBuf,
    pub login: Option<String>,
    pub config_key: String,
}

pub fn normalize_name(value: &str) -> Result<String> {
    let name = value.trim().to_ascii_lowercase();
    ensure!(
        !name.is_empty()
            && name.len() <= 32
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "Неверное имя аккаунта"
    );
    Ok(name)
}

fn directory(config: &Path) -> PathBuf {
    let mut name = config.as_os_str().to_os_string();
    name.push(".accounts");
    PathBuf::from(name)
}

pub fn path(config: &Path, login: &str) -> Result<PathBuf> {
    let normalized = config::normalize_channels(&[login.into()])?;
    ensure!(normalized.len() == 1, "Укажите один логин Twitch");
    Ok(directory(config).join(format!("{}.auth.json", normalized[0])))
}

fn session_paths(config: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut paths = Vec::new();
    let legacy = config::auth_path(config);
    if legacy.is_file() {
        paths.push(("default".into(), legacy));
    }
    let dir = directory(config);
    if dir.exists() {
        for entry in fs::read_dir(dir).context("Не удалось прочитать аккаунты")?
        {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if let Some(key) = filename.strip_suffix(".auth.json")
                && normalize_name(key).is_ok()
            {
                paths.push((key.into(), entry.path()));
            }
        }
    }
    paths.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(paths)
}

pub fn discover(config: &Path) -> Result<Vec<Profile>> {
    let mut seen = HashSet::new();
    let mut profiles = Vec::new();
    for (key, path) in session_paths(config)? {
        let session = auth::read(&path).ok();
        let login = session.as_ref().and_then(|a| a.login.clone());
        if let Some(id) = session.as_ref().and_then(|a| a.user_id.as_ref())
            && !seen.insert(id.clone())
        {
            continue;
        }
        let name = login
            .clone()
            .unwrap_or_else(|| format!("Сессия {}", profiles.len() + 1));
        profiles.push(Profile {
            name,
            path,
            login,
            config_key: key,
        });
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

// Resolve sessions created before metadata was saved. Corrupt/expired sessions
// stay visible and can be removed or authorized again through the interface.
pub async fn prepare(config_path: &Path, config: &mut Config) -> Result<Vec<Profile>> {
    let client = twitch::http_client()?;
    for (_, path) in session_paths(config_path)? {
        if let Ok(session) = auth::read(&path)
            && (session.login.is_none() || session.user_id.is_none())
            && let Ok(identity) = auth::validate(&client, &session).await
        {
            let mut session = session;
            auth::sync_identity(&mut session, &identity, &path)?;
        }
    }
    let mut profiles = discover(config_path)?;
    let mut changed = false;
    for profile in &mut profiles {
        if profile.login.is_some() && profile.config_key != profile.name {
            if let Some(channels) = config.account_channels.remove(&profile.config_key) {
                config
                    .account_channels
                    .entry(profile.name.clone())
                    .or_insert(channels);
                changed = true;
            }
            if let Some(enabled) = &mut config.enabled_accounts {
                for name in enabled {
                    if *name == profile.config_key {
                        *name = profile.name.clone();
                        changed = true;
                    }
                }
            }
        }
        // Known accounts use their Twitch login for settings, even when the
        // token remains at a legacy path.
        if profile.login.is_some() {
            profile.config_key = profile.name.clone();
        }
    }
    if changed {
        config.save(config_path)?;
    }
    Ok(profiles)
}

pub fn select(available: &[Profile], requested: &[String]) -> Result<Vec<Profile>> {
    if requested.is_empty() {
        ensure!(
            !available.is_empty(),
            "Нет аккаунтов. Добавьте аккаунт через интерфейс или twfarmer login."
        );
        return Ok(available.to_vec());
    }
    let mut profiles = Vec::new();
    let mut seen = HashSet::new();
    for login in requested {
        let requested = login.trim();
        let profile = available
            .iter()
            .find(|p| {
                p.name.eq_ignore_ascii_case(requested)
                    || p.login
                        .as_ref()
                        .is_some_and(|login| login.eq_ignore_ascii_case(requested))
            })
            .with_context(|| format!("Аккаунт {login} не найден. Выполните twfarmer login."))?;
        if seen.insert(profile.name.clone()) {
            profiles.push(profile.clone());
        }
    }
    Ok(profiles)
}

fn create_directory(config: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(directory(config))
        .context("Не удалось создать каталог аккаунтов")
}

pub fn save_session(
    config_path: &Path,
    session: &Auth,
    identity: &Identity,
    expected: Option<&Profile>,
) -> Result<String> {
    if let Some(expected) = expected {
        let old_id = auth::read(&expected.path).ok().and_then(|a| a.user_id);
        ensure!(
            old_id.as_deref().map_or_else(
                || expected
                    .login
                    .as_ref()
                    .is_none_or(|login| login == &identity.login),
                |id| id == identity.user_id
            ),
            "В браузере выбран другой аккаунт. Войдите как {}.",
            expected.name
        );
    }
    let existing = discover(config_path)?.into_iter().find(|p| {
        auth::read(&p.path)
            .ok()
            .is_some_and(|a| a.user_id.as_deref() == Some(&identity.user_id))
    });
    let target = expected
        .map(|p| p.path.clone())
        .or_else(|| existing.map(|p| p.path))
        .unwrap_or(path(config_path, &identity.login)?);
    create_directory(config_path)?;
    auth::save(&target, session)?;
    Ok(identity.login.clone())
}

pub async fn login(
    config_path: &Path,
    config: &Config,
    expected: Option<&Profile>,
) -> Result<String> {
    println!("В браузере выберите нужный аккаунт Twitch.");
    let (session, identity) = auth::authorize(&twitch::http_client()?, config).await?;
    let login = save_session(config_path, &session, &identity, expected)?;
    println!("Аккаунт {login} сохранён.");
    Ok(login)
}

pub fn remove(config_path: &Path, profile: &Profile) -> Result<()> {
    let id = auth::read(&profile.path).ok().and_then(|a| a.user_id);
    // Removing an account also removes duplicate local copies of its token.
    for (_, path) in session_paths(config_path)? {
        let same = path == profile.path
            || id.as_ref().is_some_and(|id| {
                auth::read(&path).ok().and_then(|a| a.user_id).as_ref() == Some(id)
            });
        if same {
            fs::remove_file(path).context("Не удалось удалить сессию")?;
        }
    }
    Ok(())
}

pub fn remove_and_save(config_path: &Path, config: &mut Config, profile: &Profile) -> Result<()> {
    remove(config_path, profile)?;
    config.account_channels.remove(&profile.config_key);
    if let Some(selected) = &mut config.enabled_accounts {
        selected.retain(|name| name != &profile.name);
    }
    config.save(config_path)?;
    Ok(())
}

pub fn select_and_save(
    config_path: &Path,
    config: &mut Config,
    profiles: &[Profile],
    logins: &[String],
    all: bool,
) -> Result<()> {
    config.enabled_accounts = if all {
        None
    } else if logins.is_empty() {
        Some(vec![])
    } else {
        Some(
            select(profiles, logins)?
                .into_iter()
                .map(|p| p.name)
                .collect(),
        )
    };
    config.save(config_path)
}

pub fn set_channels(config: &mut Config, profile: &Profile, values: &[String]) -> Result<()> {
    let channels = config::normalize_channels(values)?;
    if channels.is_empty() {
        config.account_channels.remove(&profile.config_key);
    } else {
        config
            .account_channels
            .insert(profile.config_key.clone(), channels);
    }
    Ok(())
}

pub fn list(profiles: &[Profile], config: &Config) {
    if profiles.is_empty() {
        println!("Сохранённых аккаунтов нет.");
        return;
    }
    println!("{:<26} КАНАЛЫ", "TWITCH");
    for profile in profiles {
        let source = if config.account_channels.contains_key(&profile.config_key) {
            "свои"
        } else {
            "общие"
        };
        println!(
            "{:<26} {}: {}",
            profile.name,
            source,
            config.channels_for(&profile.config_key).join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(login: &str, id: &str) -> Auth {
        Auth {
            access_token: "test-only".into(),
            refresh_token: None,
            client_id: "test".into(),
            device_id: "device".into(),
            login: Some(login.into()),
            user_id: Some(id.into()),
        }
    }
    #[test]
    fn paths_use_twitch_login_and_cannot_escape_the_directory() {
        assert_eq!(
            path(Path::new("/tmp/config.toml"), "TWITCH_USER").unwrap(),
            Path::new("/tmp/config.toml.accounts/twitch_user.auth.json")
        );
        for invalid in ["../secret", "a/b", "", "name.auth.json", "a-b"] {
            assert!(path(Path::new("/tmp/config.toml"), invalid).is_err());
        }
    }
    #[tokio::test]
    async fn legacy_aliases_become_logins_and_keep_own_lists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        auth::save(&config::auth_path(&file), &session("real_user", "123")).unwrap();
        let mut config = Config::default();
        config
            .account_channels
            .insert("default".into(), vec!["own".into()]);
        config.enabled_accounts = Some(vec!["default".into()]);
        config.save(&file).unwrap();
        let profiles = prepare(&file, &mut config).await.unwrap();
        assert_eq!(profiles[0].name, "real_user");
        assert_eq!(config.channels_for("real_user"), ["own"]);
        assert!(!config.account_channels.contains_key("default"));
        assert_eq!(config.enabled_accounts.unwrap(), ["real_user"]);
        assert!(select(&profiles, &["default".into()]).is_err());
        assert_eq!(select(&profiles, &["REAL_USER".into()]).unwrap().len(), 1);
    }
    #[test]
    fn renewal_rejects_the_wrong_user_and_reuses_the_same_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let identity = Identity {
            client_id: "test".into(),
            login: "real_user".into(),
            user_id: "123".into(),
            expires_in: 0,
        };
        save_session(&file, &session("real_user", "123"), &identity, None).unwrap();
        let profiles = discover(&file).unwrap();
        let other = Identity {
            client_id: "test".into(),
            login: "other".into(),
            user_id: "456".into(),
            expires_in: 0,
        };
        assert!(save_session(&file, &session("other", "456"), &other, Some(&profiles[0])).is_err());
        save_session(&file, &session("real_user", "123"), &identity, None).unwrap();
        assert_eq!(discover(&file).unwrap().len(), 1);
        assert_eq!(
            auth::read(&profiles[0].path).unwrap().user_id.as_deref(),
            Some("123")
        );
    }
}
