use crate::{
    accounts::{self, Profile},
    auth::{self, Auth, Identity},
    config::Config,
    control::{self, Action, Controls, EditTarget, Overlay},
    farmer::{self, ActiveUsers},
    local::{self, Request, Server},
    model::{AccountStatus, ChannelStatus, ChannelView, Dashboard, Event, Reporter},
    twitch,
    ui::{Screen, ShellView, ViewState},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event as TerminalEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use tokio::{
    sync::mpsc,
    task::JoinSet,
    time::{Duration, MissedTickBehavior, interval},
};

enum Job {
    Campaigns {
        results: Vec<(String, Result<Vec<crate::drops::CatalogEntry>>)>,
    },
    Loaded {
        path: PathBuf,
        result: Result<(Config, Vec<Profile>)>,
        start: bool,
        requested: Vec<String>,
    },
    Authorized {
        result: Result<(Auth, Identity)>,
        expected: Option<Profile>,
    },
}
struct Prompt {
    generation: u64,
    uri: String,
    code: String,
}

struct App {
    path: PathBuf,
    config: Config,
    profiles: Vec<Profile>,
    selected: HashSet<String>,
    temporary: Vec<String>,
    model: Dashboard,
    view: ViewState,
    controls: Controls,
    workers: JoinSet<()>,
    jobs: JoinSet<(u64, Job)>,
    events_tx: mpsc::UnboundedSender<Event>,
    events_rx: mpsc::UnboundedReceiver<Event>,
    prompt_tx: mpsc::UnboundedSender<Prompt>,
    prompt_rx: mpsc::UnboundedReceiver<Prompt>,
    anchors: HashMap<(String, String), ChannelView>,
    running: bool,
    generation: u64,
    server: Option<Server>,
    use_saved_selection: bool,
    configured_only: bool,
}

impl App {
    fn new(path: PathBuf) -> Self {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
        Self {
            path,
            config: Config::default(),
            profiles: vec![],
            selected: HashSet::new(),
            temporary: vec![],
            model: Dashboard::new(vec![]),
            view: ViewState::default(),
            controls: Controls::default(),
            workers: JoinSet::new(),
            jobs: JoinSet::new(),
            events_tx,
            events_rx,
            prompt_tx,
            prompt_rx,
            anchors: HashMap::new(),
            running: false,
            generation: 0,
            server: None,
            use_saved_selection: false,
            configured_only: false,
        }
    }

    fn log(&mut self, message: impl Into<String>, error: bool) {
        let message = message.into();
        if self.controls.plain_log {
            println!("{message}");
        }
        self.controls.notice = message.clone();
        self.model.apply(Event::Log {
            profile: "TwFarmer".into(),
            message,
            error,
        });
    }

    fn apply(&mut self, mut event: Event) {
        if self.controls.plain_log
            && let Event::Log {
                profile,
                message,
                error,
            } = &event
        {
            if *error {
                eprintln!("[{profile}] {message}");
            } else {
                println!("[{profile}] {message}");
            }
        }
        if let Event::Channel { profile, view } = &mut event
            && let Some(previous) = self
                .model
                .accounts
                .iter()
                .find(|a| a.name == *profile)
                .and_then(|a| a.channels.iter().find(|c| c.name == view.name))
        {
            if view.balance.is_none() {
                view.balance = previous.balance;
                view.baseline = previous.baseline;
                view.last_update = previous.last_update;
            }
            if view.last_success.is_none() {
                view.last_success = previous.last_success;
            }
        }
        if let Event::Channel { profile, view } = &mut event
            && let Some(anchor) = self.anchors.get(&(profile.clone(), view.name.clone()))
        {
            if anchor.baseline.is_some() {
                view.baseline = anchor.baseline;
            }
            view.bonuses += anchor.bonuses;
            view.active_time += anchor.active_time;
        }
        self.model.apply(event);
    }

    fn runtime_config(&self) -> Result<Config> {
        let mut config = self.config.clone();
        config.override_for_run(&self.temporary)?;
        Ok(config)
    }

    fn rebuild(&mut self) -> Result<()> {
        let config = self.runtime_config()?;
        let mut old = std::mem::take(&mut self.model.accounts);
        let mut next = Dashboard::new(
            self.profiles
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        p.login.clone(),
                        config.channels_for(&p.config_key).to_vec(),
                    )
                })
                .collect(),
        );
        for account in &mut next.accounts {
            account.status = AccountStatus::Idle;
            for channel in &mut account.channels {
                channel.status = ChannelStatus::Idle;
            }
            if let Some(previous) = old.iter_mut().find(|a| a.name == account.name) {
                account.status = previous.status;
                account.error = previous.error.take();
                for channel in &mut account.channels {
                    if let Some(index) = previous
                        .channels
                        .iter()
                        .position(|c| c.name == channel.name)
                    {
                        *channel = previous.channels.remove(index);
                    }
                }
            }
        }
        self.model.accounts = next.accounts;
        self.view = ViewState::default();
        Ok(())
    }

    async fn stop(&mut self) {
        self.workers.abort_all();
        while self.workers.join_next().await.is_some() {}
        while let Ok(event) = self.events_rx.try_recv() {
            self.apply(event);
        }
        self.running = false;
        for account in &mut self.model.accounts {
            if account.status != AccountStatus::Error {
                account.status = AccountStatus::Idle;
            }
            for channel in &mut account.channels {
                channel.farm_role = crate::model::FarmRole::None;
                channel.chat_status = crate::model::ChatStatus::Disabled;
                channel.chat_error = None;
                if matches!(
                    channel.status,
                    ChannelStatus::Watching
                        | ChannelStatus::Starting
                        | ChannelStatus::Retry(_)
                        | ChannelStatus::Checking
                ) {
                    channel.status = ChannelStatus::Idle;
                }
            }
        }
    }

    async fn launch(&mut self, status_only: bool) -> Result<()> {
        ensure!(
            !self.selected.is_empty(),
            "{}",
            tf!(
                "Select at least one account in the menu.",
                "Выберите хотя бы один аккаунт в меню."
            )
        );
        let profiles: Vec<_> = self
            .profiles
            .iter()
            .filter(|p| self.selected.contains(&p.name))
            .cloned()
            .collect();
        ensure!(
            !profiles.is_empty(),
            "{}",
            tf!(
                "Add a Twitch account first.",
                "Сначала добавьте аккаунт Twitch."
            )
        );
        let config = self.runtime_config()?;
        if !status_only {
            ensure!(
                config.automatic_drops()
                    || profiles
                        .iter()
                        .any(|p| !config.channels_for(&p.config_key).is_empty()),
                "{}",
                tf!(
                    "The channel list is empty. Set shared or personal channels in the menu.",
                    "Список каналов пуст. Задайте общие или личные каналы в меню."
                )
            );
        }
        self.stop().await;
        self.rebuild()?;
        self.anchors = self
            .model
            .accounts
            .iter()
            .flat_map(|a| {
                a.channels
                    .iter()
                    .map(move |c| ((a.name.clone(), c.name.clone()), c.clone()))
            })
            .collect();
        let users = ActiveUsers::default();
        for profile in profiles {
            let reporter = Reporter::new(profile.name.clone(), self.events_tx.clone());
            reporter.account(None, AccountStatus::Connecting, None);
            let config = config.clone();
            let users = users.clone();
            self.workers.spawn(async move {
                let result = if status_only {
                    farmer::status(profile, config, reporter.clone()).await
                } else {
                    farmer::run(profile, config, reporter.clone(), users).await
                };
                if let Err(error) = result {
                    let message = format!("{error:#}");
                    reporter.account(None, AccountStatus::Error, Some(message.clone()));
                    reporter.log(message, true);
                }
            });
        }
        self.running = !status_only;
        self.log(
            if status_only {
                tr!(
                    "Checking balances for the selected accounts.",
                    "Проверяю балансы выбранных аккаунтов."
                )
            } else {
                tr!(
                    "Farming started for the selected accounts.",
                    "Фарм запущен для выбранных аккаунтов."
                )
            },
            false,
        );
        Ok(())
    }

    fn load(&mut self, path: PathBuf, start: bool, requested: Vec<String>) {
        self.jobs.abort_all();
        self.generation += 1;
        self.controls.overlay = Some(Overlay::Loading);
        let generation = self.generation;
        let configured_only = self.configured_only;
        self.jobs.spawn(async move {
            let result = async {
                let mut config = if configured_only || path.exists() {
                    Config::read(&path)?
                } else {
                    Config::default()
                };
                let profiles = accounts::prepare(&path, &mut config).await?;
                Ok((config, profiles))
            }
            .await;
            (
                generation,
                Job::Loaded {
                    path,
                    result,
                    start,
                    requested,
                },
            )
        });
    }

    fn begin_login(&mut self, name: Option<String>) -> Result<()> {
        ensure!(
            !self.configured_only,
            "{}",
            tf!(
                "Login is disabled in daemon mode. Add a session separately with twfarmer login.",
                "В служебном режиме вход отключён. Добавьте сессию командой twfarmer login отдельно."
            )
        );
        let expected = name.map(|name| self.profile(&name)).transpose()?;
        self.jobs.abort_all();
        self.generation += 1;
        let generation = self.generation;
        let sender = self.prompt_tx.clone();
        let config = self.config.clone();
        self.controls.overlay = Some(Overlay::Login {
            uri: String::new(),
            code: String::new(),
        });
        self.jobs.spawn(async move {
            let result = async {
                let client = twitch::http_client()?;
                auth::authorize_with(&client, &config, move |uri, code| {
                    let _ = sender.send(Prompt {
                        generation,
                        uri,
                        code,
                    });
                })
                .await
            }
            .await;
            (generation, Job::Authorized { result, expected })
        });
        Ok(())
    }

    fn profile(&self, name: &str) -> Result<Profile> {
        self.profiles
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .context(tr!("Account not found", "Аккаунт не найден"))
    }

    async fn commit(&mut self, next: Config) -> Result<()> {
        next.validate()?;
        next.save(&self.path)?;
        if self.config.language != next.language {
            crate::i18n::set_override(None);
        }
        let restart = self.running;
        if restart {
            self.stop().await;
        }
        self.config = next;
        crate::i18n::apply_config(self.config.language);
        self.rebuild()?;
        if restart && !self.selected.is_empty() {
            self.launch(false).await?;
        }
        self.controls.overlay = None;
        self.log(tr!("Settings saved.", "Настройки сохранены."), false);
        Ok(())
    }

    async fn action(&mut self, action: Action) -> Result<bool> {
        let save_selection = matches!(&action, Action::SaveSelection(_));
        match action {
            Action::Campaigns => {
                let profiles: Vec<_> = self
                    .profiles
                    .iter()
                    .filter(|p| self.selected.contains(&p.name))
                    .cloned()
                    .collect();
                ensure!(
                    !profiles.is_empty(),
                    "{}",
                    tr!(
                        "Select at least one account first.",
                        "Сначала выберите хотя бы один аккаунт."
                    )
                );
                self.jobs.abort_all();
                self.generation += 1;
                let generation = self.generation;
                let config = self.config.clone();
                self.controls.overlay = Some(Overlay::CampaignsLoading);
                self.jobs.spawn(async move {
                    let mut results = vec![];
                    for profile in profiles {
                        results.push((
                            profile.name.clone(),
                            farmer::campaigns(profile, &config).await,
                        ));
                    }
                    (generation, Job::Campaigns { results })
                });
            }
            Action::SetLanguage(language) => {
                let mut next = self.config.clone();
                next.language = language;
                next.save(&self.path)?;
                self.config = next;
                crate::i18n::set_override(None);
                crate::i18n::apply_config(language);
                self.controls.overlay = None;
                self.log(
                    tf!("Language saved: {}.", "Язык сохранён: {}.", language.name()),
                    false,
                );
            }
            Action::Quit => return Ok(true),
            Action::PlainLog => {
                if !self.running {
                    self.launch(false).await?;
                }
                self.controls.overlay = None;
                self.controls.plain_log = true;
            }
            Action::Reload => {
                let restart = self.running;
                Config::read(&self.path)?;
                self.stop().await;
                self.load(self.path.clone(), restart, vec![]);
            }
            Action::CancelJob => {
                self.jobs.abort_all();
                self.generation += 1;
                self.log(tr!("Operation cancelled.", "Операция отменена."), false);
            }
            Action::Start => self.launch(false).await?,
            Action::Stop => {
                self.stop().await;
                self.log(tr!("Farming stopped.", "Фарм остановлен."), false);
            }
            Action::Status => self.launch(true).await?,
            Action::ToggleLog => {
                self.controls.log_only = !self.controls.log_only;
            }
            Action::Login(name) => self.begin_login(name)?,
            Action::Remove(name) => {
                let profile = self.profile(&name)?;
                let restart = self.running;
                self.stop().await;
                accounts::remove_and_save(&self.path, &mut self.config, &profile)?;
                self.selected.remove(&name);
                self.config.account_channels.remove(&profile.config_key);
                self.config.enabled_accounts = Some(self.selected.iter().cloned().collect());
                self.config.save(&self.path)?;
                let path = self.path.clone();
                self.load(path, restart, vec![]);
            }
            Action::SaveSelection(chosen) | Action::UseSelection(chosen) => {
                let restart = self.running;
                let mut next = self.config.clone();
                let mut selected: Vec<_> = chosen
                    .iter()
                    .filter(|name| self.profiles.iter().any(|p| p.name == **name))
                    .cloned()
                    .collect();
                selected.sort();
                if save_selection {
                    next.enabled_accounts = Some(selected);
                    next.save(&self.path)?;
                }
                self.stop().await;
                self.selected = chosen;
                self.config = next;
                self.controls.overlay = None;
                if restart && !self.selected.is_empty() {
                    self.launch(false).await?;
                }
                self.log(
                    tf!(
                        "Accounts selected: {}.",
                        "Выбрано аккаунтов: {}.",
                        self.selected.len()
                    ),
                    false,
                );
            }
            Action::Inherit(name) => {
                let profile = self.profile(&name)?;
                let mut next = self.config.clone();
                accounts::set_channels(&mut next, &profile, &[])?;
                self.commit(next).await?;
            }
            Action::SaveSettings(values) => {
                let next = control::settings(&self.config, &values)?;
                self.commit(next).await?;
            }
            Action::SaveEdit(target, text) => {
                match target {
                    EditTarget::Config => {
                        let path = control::config_path(&text)?;
                        if !path.exists() {
                            Config::default().save(&path)?;
                        }
                        Config::read(&path)?;
                        self.stop().await;
                        self.temporary.clear();
                        self.load(path, false, vec![]);
                    }
                    EditTarget::Temporary => {
                        let channels = crate::config::normalize_channels(&[text])?;
                        let restart = self.running;
                        if restart {
                            self.stop().await;
                        }
                        self.temporary = channels;
                        self.rebuild()?;
                        self.controls.overlay = None;
                        if restart {
                            self.launch(false).await?;
                        }
                        self.log(
                        tr!("Temporary channels updated. Empty input restores the saved lists.", "Временный список изменён. Пустой ввод возвращает сохранённые списки."),
                        false,
                    );
                    }
                    EditTarget::Common => {
                        let mut next = self.config.clone();
                        next.channels = crate::config::normalize_channels(&[text])?;
                        self.commit(next).await?;
                    }
                    EditTarget::Own(name) => {
                        let profile = self.profile(&name)?;
                        let mut next = self.config.clone();
                        accounts::set_channels(&mut next, &profile, &[text])?;
                        self.commit(next).await?;
                    }
                }
            }
        }
        Ok(false)
    }

    async fn job(&mut self, job: Job) -> Result<()> {
        match job {
            Job::Campaigns { results } => {
                let mut rows = vec![];
                let mut errors = vec![];
                for (name, result) in results {
                    match result {
                        Ok(entries) => {
                            rows.extend(entries.iter().cloned().map(|entry| (name.clone(), entry)));
                            self.apply(Event::Campaigns {
                                profile: name,
                                entries,
                                error: None,
                            });
                        }
                        Err(error) => errors.push(format!("[{name}] {error:#}")),
                    }
                }
                self.controls.overlay = Some(Overlay::Campaigns {
                    index: 0,
                    entries: rows,
                    errors,
                    show_all: false,
                    expanded: false,
                    scroll: 0,
                });
            }
            Job::Loaded {
                path,
                result,
                start,
                requested,
            } => {
                let (config, profiles) = result?;
                let keep_selection = if !self.use_saved_selection
                    && !self.profiles.is_empty()
                    && self.path == path
                    && self.config.enabled_accounts == config.enabled_accounts
                {
                    Some(self.selected.clone())
                } else {
                    None
                };
                if self
                    .server
                    .as_ref()
                    .is_some_and(|server| local::key(&path).ok().as_ref() != Some(&server.key))
                {
                    let server = Server::bind(&path).await?;
                    self.server = Some(server);
                }
                self.use_saved_selection = false;
                self.path = path;
                self.config = config;
                crate::i18n::apply_config(self.config.language);
                self.profiles = profiles;
                if self.configured_only {
                    ensure!(
                        !self.profiles.is_empty(),
                        "{}",
                        tf!(
                            "No saved sessions. Run twfarmer login with this --config path first.",
                            "Нет сохранённых сессий. Сначала выполните twfarmer login с этим путём --config."
                        )
                    );
                }
                self.selected = if !requested.is_empty() {
                    accounts::select(&self.profiles, &requested)?
                        .into_iter()
                        .map(|p| p.name)
                        .collect()
                } else if let Some(selected) = keep_selection {
                    self.profiles
                        .iter()
                        .filter(|p| selected.contains(&p.name))
                        .map(|p| p.name.clone())
                        .collect()
                } else if let Some(enabled) = &self.config.enabled_accounts {
                    self.profiles
                        .iter()
                        .filter(|p| enabled.contains(&p.name))
                        .map(|p| p.name.clone())
                        .collect()
                } else {
                    self.profiles.iter().map(|p| p.name.clone()).collect()
                };
                self.controls.overlay = None;
                self.rebuild()?;
                if start {
                    ensure!(
                        !self.selected.is_empty(),
                        "{}",
                        tf!(
                            "No accounts selected. Add an account or select accounts to start.",
                            "Нет выбранных аккаунтов. Добавьте аккаунт или выберите аккаунты для запуска."
                        )
                    );
                    self.launch(false).await?;
                } else {
                    self.controls.menu();
                    self.log(
                        if self.profiles.is_empty() {
                            tr!(
                                "Add a Twitch account and set channels in the menu.",
                                "Добавьте аккаунт Twitch и задайте каналы через меню."
                            )
                        } else {
                            tr!(
                                "Choose an action in the menu. Enter to run, Esc to close.",
                                "Выберите действие в меню. Enter для запуска, Esc для закрытия."
                            )
                        },
                        false,
                    );
                }
            }
            Job::Authorized { result, expected } => {
                let (session, identity) = result?;
                let restart = self.running;
                self.stop().await;
                let login = match accounts::save_session(
                    &self.path,
                    &session,
                    &identity,
                    expected.as_ref(),
                ) {
                    Ok(login) => login,
                    Err(error) => {
                        if restart {
                            self.launch(false).await?;
                        }
                        return Err(error);
                    }
                };
                self.selected.insert(login.clone());
                self.config.enabled_accounts = Some(self.selected.iter().cloned().collect());
                self.config.save(&self.path)?;
                self.log(tf!("Signed in: {login}.", "Вход выполнен: {login}."), false);
                self.load(self.path.clone(), restart, vec![]);
            }
        }
        Ok(())
    }
    async fn remote(&mut self, request: Request) -> Result<bool> {
        match request {
            Request::Start { accounts, channels } => {
                ensure!(
                    !self.configured_only || channels.is_empty(),
                    "{}",
                    tf!(
                        "Daemon mode uses configured channels. Save the list with channels and reload the configuration.",
                        "В служебном режиме каналы берутся из конфигурации. Сохраните список командой channels и перечитайте настройки."
                    )
                );
                ensure!(
                    !matches!(self.controls.overlay, Some(Overlay::Loading)),
                    "{}",
                    tf!(
                        "Wait for the settings to load, then run start again.",
                        "Дождитесь загрузки настроек и повторите start."
                    )
                );
                if !accounts.is_empty() {
                    self.selected = accounts::select(&self.profiles, &accounts)?
                        .into_iter()
                        .map(|p| p.name)
                        .collect();
                }
                self.temporary = crate::config::normalize_channels(&channels)?;
                self.launch(false).await?;
                self.controls.overlay = None;
            }
            Request::Stop => {
                self.action(Action::Stop).await?;
                return Ok(self.controls.plain_log);
            }
            Request::Quit => return Ok(true),
            Request::ReloadSelection => {
                self.use_saved_selection = true;
                self.action(Action::Reload).await?;
            }
            Request::Reload => {
                self.action(Action::Reload).await?;
            }
            Request::View { journal } => {
                ensure!(
                    !self.configured_only,
                    "{}",
                    tf!(
                        "The interface is disabled in daemon mode. Read the logs with journalctl.",
                        "В служебном режиме интерфейс отключён. Журнал доступен через journalctl."
                    )
                );
                use std::io::IsTerminal;
                ensure!(
                    std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
                    "{}",
                    tf!(
                        "Run the program in an interactive terminal to use the interface.",
                        "Для интерфейса запустите программу в интерактивном терминале."
                    )
                );
                self.controls.plain_log = false;
                self.controls.log_only = journal;
                self.controls.overlay = None;
            }
        }
        Ok(false)
    }
}

pub async fn run(
    path: PathBuf,
    start: bool,
    requested: Vec<String>,
    channels: Vec<String>,
    journal: bool,
    no_ui: bool,
) -> Result<()> {
    run_with_options(
        path,
        RunOptions {
            start,
            requested,
            channels,
            journal,
            no_ui,
            configured_only: false,
        },
    )
    .await
}

pub async fn daemon(path: PathBuf, requested: Vec<String>) -> Result<()> {
    // Validate configuration before creating control files.
    Config::read(&path)?;
    run_with_options(
        path,
        RunOptions {
            start: true,
            requested,
            channels: vec![],
            journal: false,
            no_ui: true,
            configured_only: true,
        },
    )
    .await
}

struct RunOptions {
    start: bool,
    requested: Vec<String>,
    channels: Vec<String>,
    journal: bool,
    no_ui: bool,
    configured_only: bool,
}

async fn run_with_options(path: PathBuf, options: RunOptions) -> Result<()> {
    let RunOptions {
        start,
        requested,
        channels,
        journal,
        no_ui,
        configured_only,
    } = options;
    let mut screen = if no_ui { None } else { Some(Screen::new()?) };
    let mut app = App::new(path.clone());
    app.configured_only = configured_only;
    app.server = Some(Server::bind(&path).await?);
    app.controls.log_only = journal;
    app.controls.plain_log = no_ui;
    app.temporary = crate::config::normalize_channels(&channels)?;
    if configured_only {
        println!(
            "{}",
            tf!(
                "Daemon mode. Configuration: {}. Saved sessions only; SIGTERM / Ctrl+C to stop.",
                "Служебный режим. Конфигурация: {}. Только сохранённые сессии; SIGTERM / Ctrl+C для остановки.",
                local::key(&path)?.display()
            )
        );
    }
    let mut input = if no_ui {
        None
    } else {
        Some(EventStream::new())
    };
    let mut ticker = interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    app.load(path, start, requested);
    loop {
        if app.controls.plain_log && screen.is_some() {
            drop(screen.take());
            input = None;
            println!(
                "{}",
                tf!(
                    "Plain log. Ctrl+C or twfarmer stop to stop farming.",
                    "Обычный журнал. Ctrl+C или twfarmer stop для остановки."
                )
            );
        } else if !app.controls.plain_log && screen.is_none() {
            screen = Some(Screen::new()?);
            input = Some(EventStream::new());
        }
        if app.controls.plain_log && app.running && app.workers.is_empty() && app.jobs.is_empty() {
            while let Ok(event) = app.events_rx.try_recv() {
                app.apply(event);
            }
            anyhow::bail!(
                "{}",
                tf!(
                    "No accounts are still running. Check the session errors.",
                    "Не осталось работающих аккаунтов. Проверьте ошибки сессий."
                )
            );
        }
        tokio::select! {
            _=ticker.tick() => if let Some(screen)=&mut screen {
                screen.draw_shell(&app.model,&mut app.view,ShellView {
                    controls:&app.controls,config:&app.config,profiles:&app.profiles,selected:&app.selected,
                    path:&app.path,temporary:&app.temporary,
                })?;
            },
            Some(event)=app.events_rx.recv() => app.apply(event),
            Some(prompt)=app.prompt_rx.recv() => {
                if prompt.generation==app.generation {
                    app.controls.overlay=Some(Overlay::Login {uri:prompt.uri,code:prompt.code});
                }
            }
            Some(result)=app.jobs.join_next(), if !app.jobs.is_empty() => {
                let result=match result {
                    Ok((generation,job)) if generation==app.generation=>app.job(job).await,
                    Ok(_)=>Ok(()),Err(e) if e.is_cancelled()=>Ok(()),Err(e)=>Err(e.into()),
                };
                if let Err(error)=result {
                    if app.controls.plain_log {return Err(error);}
                    app.controls.overlay=None;app.log(format!("{error:#}"),true);
                }
            }
            Some(result)=app.workers.join_next(), if !app.workers.is_empty() => {
                if let Err(error)=result && !error.is_cancelled() {app.log(tf!("Task error: {error}", "Ошибка задачи: {error}"),true);}
                if app.controls.plain_log && app.workers.is_empty() && app.running {
                    while let Ok(event)=app.events_rx.try_recv() {app.apply(event);}
                    anyhow::bail!("{}", tf!("No accounts are still running. Check the session errors.", "Не осталось работающих аккаунтов. Проверьте ошибки сессий."));
                }
            }
            connection=async {
                match &app.server {Some(server)=>server.accept().await,None=>std::future::pending().await}
            } => match connection {
                Ok(mut connection) => {
                    let result=match connection.request().await {
                        Ok(request)=>app.remote(request).await,Err(error)=>Err(error),
                    };
                    let quit=matches!(result,Ok(true));
                    let reply=result.map(|_|tr!("Command completed.", "Команда выполнена.").to_owned());
                    if let Err(error)=connection.reply(reply).await {app.log(tf!("Control: {error:#}", "Управление: {error:#}"),true);}
                    if quit {break;}
                }
                Err(error)=>app.log(tf!("Control: {error:#}", "Управление: {error:#}"),true),
            },
            event=async {
                match &mut input {Some(input)=>input.next().await,None=>std::future::pending().await}
            } => match event {
                Some(Ok(TerminalEvent::Key(key))) if key.kind!=KeyEventKind::Release => {
                    if key.code==KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {break;}
                    let had_overlay=app.controls.overlay.is_some();
                    if let Some(action)=app.controls.key(key,&app.profiles,&app.config,&app.selected) {
                        match app.action(action).await {
                            Ok(true)=>break,Ok(false)=>{},Err(error)=>app.log(format!("{error:#}"),true),
                        }
                    } else if !had_overlay && app.controls.overlay.is_none() {
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k')=>app.view.move_selection(&app.model,-1),
                            KeyCode::Down | KeyCode::Char('j')=>app.view.move_selection(&app.model,1),
                            KeyCode::PageUp=>app.view.move_selection(&app.model,-10),
                            KeyCode::PageDown=>app.view.move_selection(&app.model,10),
                            KeyCode::Tab=>app.view.next_account(&app.model),
                            _=>{},
                        }
                    }
                }
                Some(Ok(TerminalEvent::Paste(text)))=>app.controls.paste(&text),
                Some(Ok(_))=>{},Some(Err(error))=>return Err(error.into()),None=>break,
            },
        }
    }
    app.jobs.abort_all();
    app.stop().await;
    drop(screen);
    println!(
        "{}",
        tf!(
            "Stopped. Balance change: {:+}. Bonuses: {}.",
            "Остановлено. Изменение баланса: {:+}. Бонусов: {}.",
            app.model.gain(),
            app.model.bonuses()
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn daemon_requires_an_existing_configuration_without_creating_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        assert!(daemon(path.clone(), vec![]).await.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn daemon_cannot_open_login_ui_or_override_configured_channels() {
        let mut app = App::new(PathBuf::from("/tmp/config.toml"));
        app.configured_only = true;
        app.controls.plain_log = true;
        assert!(app.begin_login(None).is_err());
        assert!(app.jobs.is_empty());
        assert!(app.remote(Request::View { journal: true }).await.is_err());
        assert!(app.controls.plain_log);
        assert!(
            app.remote(Request::Start {
                accounts: vec![],
                channels: vec!["new_channel".into()]
            })
            .await
            .is_err()
        );
        assert!(app.temporary.is_empty());
        assert!(
            app.job(Job::Loaded {
                path: PathBuf::from("/tmp/config.toml"),
                result: Ok((Config::default(), vec![])),
                start: true,
                requested: vec![],
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn ui_actions_save_common_own_and_selection_without_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut app = App::new(path.clone());
        app.profiles.push(Profile {
            name: "twitch_user".into(),
            login: Some("twitch_user".into()),
            path: dir.path().join("session"),
            config_key: "twitch_user".into(),
        });
        app.action(Action::SaveEdit(EditTarget::Common, "common".into()))
            .await
            .unwrap();
        app.action(Action::SaveEdit(
            EditTarget::Own("twitch_user".into()),
            "own".into(),
        ))
        .await
        .unwrap();
        assert_eq!(
            Config::read(&path).unwrap().channels_for("twitch_user"),
            ["own"]
        );
        app.action(Action::Inherit("twitch_user".into()))
            .await
            .unwrap();
        assert_eq!(
            Config::read(&path).unwrap().channels_for("twitch_user"),
            ["common"]
        );
        app.action(Action::SaveSelection(HashSet::new()))
            .await
            .unwrap();
        assert_eq!(Config::read(&path).unwrap().enabled_accounts, Some(vec![]));
        let saved = std::fs::read(&path).unwrap();
        app.action(Action::SaveEdit(EditTarget::Temporary, "temporary".into()))
            .await
            .unwrap();
        assert_eq!(
            app.runtime_config().unwrap().channels_for("twitch_user"),
            ["temporary"]
        );
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    #[tokio::test]
    async fn commands_and_forms_produce_identical_saved_settings() {
        use clap::Parser;
        let ui_dir = tempfile::tempdir().unwrap();
        let cli_dir = tempfile::tempdir().unwrap();
        let ui_path = ui_dir.path().join("config.toml");
        let cli_path = cli_dir.path().join("config.toml");
        let session = Auth {
            access_token: "test-only".into(),
            refresh_token: None,
            client_id: "test".into(),
            device_id: "test".into(),
            login: Some("user".into()),
            user_id: Some("123".into()),
        };
        auth::save(&crate::config::auth_path(&ui_path), &session).unwrap();
        auth::save(&crate::config::auth_path(&cli_path), &session).unwrap();
        let mut app = App::new(ui_path.clone());
        app.profiles = accounts::prepare(&ui_path, &mut app.config).await.unwrap();
        async fn cli(path: &std::path::Path, args: &[&str]) {
            let mut values = vec!["twfarmer", "--config", path.to_str().unwrap()];
            values.extend_from_slice(args);
            crate::execute(crate::Cli::try_parse_from(values).unwrap())
                .await
                .unwrap();
        }
        app.action(Action::SaveEdit(EditTarget::Common, "common".into()))
            .await
            .unwrap();
        cli(&cli_path, &["channels", "common"]).await;
        app.action(Action::SaveEdit(
            EditTarget::Own("user".into()),
            "own".into(),
        ))
        .await
        .unwrap();
        cli(&cli_path, &["accounts", "channels", "user", "own"]).await;
        app.action(Action::SaveSelection(HashSet::from(["user".into()])))
            .await
            .unwrap();
        cli(&cli_path, &["accounts", "select", "user"]).await;
        let values = vec![
            "1".into(),
            "90".into(),
            "20".into(),
            "false".into(),
            app.config.client_id.clone(),
            "user_read".into(),
            "en".into(),
            "true".into(),
            "false".into(),
            "false".into(),
            "true".into(),
            "drops".into(),
        ];
        app.action(Action::SaveSettings(values)).await.unwrap();
        cli(
            &cli_path,
            &[
                "settings",
                "set",
                "--max-channels",
                "1",
                "--poll-interval-seconds",
                "90",
                "--claim-bonus",
                "false",
                "--farm-drops",
                "false",
                "--claim-drops",
                "false",
                "--auto-drops",
                "true",
                "--farming-priority",
                "drops",
            ],
        )
        .await;
        app.action(Action::SaveEdit(EditTarget::Common, "".into()))
            .await
            .unwrap();
        cli(&cli_path, &["channels", "--clear"]).await;
        app.action(Action::Inherit("user".into())).await.unwrap();
        cli(&cli_path, &["accounts", "channels", "user"]).await;
        assert_eq!(
            std::fs::read_to_string(&ui_path).unwrap(),
            std::fs::read_to_string(&cli_path).unwrap()
        );
        app.action(Action::Remove("user".into())).await.unwrap();
        cli(&cli_path, &["accounts", "remove", "user"]).await;
        assert_eq!(
            std::fs::read_to_string(&ui_path).unwrap(),
            std::fs::read_to_string(&cli_path).unwrap()
        );
        assert!(!crate::config::auth_path(&ui_path).exists());
        assert!(!crate::config::auth_path(&cli_path).exists());
        app.jobs.abort_all();
    }

    #[tokio::test]
    async fn temporary_account_selection_does_not_overwrite_saved_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut app = App::new(path.clone());
        app.config.enabled_accounts = Some(vec!["one".into()]);
        app.config.save(&path).unwrap();
        app.profiles.push(Profile {
            name: "one".into(),
            login: Some("one".into()),
            path: dir.path().join("session"),
            config_key: "one".into(),
        });
        let saved = std::fs::read(&path).unwrap();
        app.action(Action::UseSelection(HashSet::new()))
            .await
            .unwrap();
        assert!(app.selected.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }

    #[test]
    fn restarting_workers_keeps_the_dashboard_session_totals() {
        let mut app = App::new(PathBuf::from("/tmp/config"));
        let mut old = ChannelView::new("channel".into());
        old.set_balance(100);
        old.set_balance(150);
        old.bonuses = 2;
        old.active_time = Duration::from_secs(30);
        app.model = Dashboard::new(vec![(
            "user".into(),
            Some("user".into()),
            vec!["channel".into()],
        )]);
        app.anchors.insert(("user".into(), "channel".into()), old);
        let mut next = ChannelView::new("channel".into());
        next.set_balance(150);
        next.set_balance(160);
        next.bonuses = 1;
        next.active_time = Duration::from_secs(20);
        app.apply(Event::Channel {
            profile: "user".into(),
            view: next,
        });
        assert_eq!(app.model.gain(), 60);
        assert_eq!(app.model.bonuses(), 3);
        assert_eq!(
            app.model.accounts[0].channels[0].active_time,
            Duration::from_secs(50)
        );
    }
}
