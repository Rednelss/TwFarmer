mod accounts;
mod app;
mod auth;
mod config;
mod control;
mod farmer;
mod hls;
mod local;
mod model;
mod tracking;
mod twitch;
mod ui;

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use config::Config;
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "twfarmer",
    version,
    about = "Twitch: фарм баллов и управление аккаунтами в консоли"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value = "twfarmer.toml",
        help = "Путь к настройкам"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    #[command(about = "Открыть интерфейс управления")]
    Ui {
        #[arg(long)]
        journal: bool,
    },
    #[command(about = "Запустить фарм в уже открытой программе")]
    Start {
        #[arg(long = "account", value_delimiter = ',')]
        accounts: Vec<String>,
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = "Остановить фарм в запущенной программе")]
    Stop,
    #[command(about = "Закрыть запущенную программу")]
    Quit,
    #[command(about = "Перечитать настройки в запущенной программе")]
    Reload,
    #[command(about = "Показать журнал или вернуть таблицу в запущенной программе")]
    Journal {
        #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
        enabled: bool,
    },
    #[command(about = "Просмотр и изменение настроек фарма")]
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    #[command(about = "Создать настройки с общими каналами")]
    Init {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = "Добавить аккаунт; имя определяется по Twitch автоматически")]
    Login {
        #[arg(long, help = "Логин Twitch для обновления существующего входа")]
        account: Option<String>,
    },
    #[command(about = "Запустить фарм; в терминале открывается интерфейс управления")]
    Run {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
        #[arg(
            long = "account",
            value_delimiter = ',',
            help = "Логины Twitch через запятую"
        )]
        accounts: Vec<String>,
        #[arg(long, help = "Обычный журнал без интерфейса")]
        no_ui: bool,
        #[arg(long, help = "Открыть журнал вместо таблицы")]
        journal: bool,
    },
    #[command(
        alias = "service",
        about = "Служебный запуск: только готовая конфигурация и сохранённые сессии"
    )]
    Daemon {
        #[arg(
            long = "account",
            value_delimiter = ',',
            help = "Сохранённые логины Twitch; иначе выбор из конфигурации"
        )]
        accounts: Vec<String>,
    },
    #[command(about = "Показать балансы аккаунтов без фарма")]
    Status {
        #[arg(long = "account", value_delimiter = ',')]
        accounts: Vec<String>,
    },
    #[command(about = "Удалить локальную сессию аккаунта")]
    Logout {
        #[arg(long, help = "Логин Twitch; можно опустить, если аккаунт один")]
        account: Option<String>,
    },
    #[command(about = "Общие каналы; без аргументов показать список")]
    Channels {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
        #[arg(long, conflicts_with = "channels", help = "Очистить общий список")]
        clear: bool,
    },
    #[command(about = "Управление аккаунтами по логину Twitch")]
    Accounts {
        #[command(subcommand)]
        command: AccountsCommand,
    },
}
#[derive(Subcommand)]
enum AccountsCommand {
    #[command(about = "Войти в аккаунт; необязательные аргументы — его личные каналы")]
    Add {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = "Показать аккаунты и списки каналов")]
    List,
    #[command(about = "Сохранить выбор аккаунтов; без логинов снять выбор")]
    Select {
        #[arg(value_name = "TWITCH_LOGINS", value_delimiter = ',')]
        logins: Vec<String>,
        #[arg(long, conflicts_with = "logins", help = "Использовать все аккаунты")]
        all: bool,
    },
    #[command(about = "Удалить локальную сессию аккаунта")]
    Remove {
        #[arg(value_name = "TWITCH_LOGIN")]
        login: String,
    },
    #[command(about = "Личные каналы аккаунта; без каналов вернуть общий список")]
    Channels {
        #[arg(value_name = "TWITCH_LOGIN")]
        login: String,
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
}

#[derive(Subcommand)]
enum SettingsCommand {
    #[command(about = "Показать настройки")]
    Show,
    #[command(about = "Изменить указанные настройки")]
    Set {
        #[command(flatten)]
        values: config::SettingsPatch,
    },
}

#[tokio::main]
async fn main() {
    let result = tokio::select! {
        result=execute(Cli::parse())=>Some(result),
        result=shutdown_signal()=>match result {Ok(())=>None,Err(error)=>Some(Err(error.into()))},
    };
    match result {
        Some(Err(error)) => {
            eprintln!("Ошибка: {error:#}");
            std::process::exit(1);
        }
        None => println!("\nОстановлено."),
        Some(Ok(())) => {}
    }
}

async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn config_or_default(path: &Path) -> Result<Config> {
    if path.exists() {
        Config::read(path)
    } else {
        Ok(Config::default())
    }
}
async fn prepared(path: &Path) -> Result<(Config, Vec<accounts::Profile>)> {
    let mut config = config_or_default(path)?;
    let profiles = accounts::prepare(path, &mut config).await?;
    Ok((config, profiles))
}
fn one(profiles: &[accounts::Profile], login: Option<String>) -> Result<accounts::Profile> {
    if let Some(login) = login {
        return accounts::select(profiles, &[login])?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Аккаунт не найден"));
    }
    ensure!(
        profiles.len() == 1,
        "Укажите логин Twitch через --account или выберите аккаунт в интерфейсе."
    );
    Ok(profiles[0].clone())
}

async fn execute(cli: Cli) -> Result<()> {
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    match cli.command {
        Some(Command::Ui { journal }) => {
            ensure!(terminal, "Интерфейсу нужен интерактивный терминал.");
            app::run(cli.config, false, vec![], vec![], journal, false).await?;
        }
        None if terminal => app::run(cli.config, false, vec![], vec![], false, false).await?,
        Some(Command::Run {
            channels,
            accounts,
            no_ui,
            journal,
        }) => {
            app::run(
                cli.config,
                true,
                accounts,
                channels,
                journal,
                no_ui || !terminal,
            )
            .await?;
        }
        Some(Command::Daemon { accounts }) => app::daemon(cli.config, accounts).await?,
        Some(Command::Start { accounts, channels }) => {
            config::normalize_channels(&channels)?;
            println!(
                "{}",
                local::send(&cli.config, local::Request::Start { accounts, channels }).await?
            );
        }
        Some(Command::Stop) => {
            println!("{}", local::send(&cli.config, local::Request::Stop).await?)
        }
        Some(Command::Quit) => {
            println!("{}", local::send(&cli.config, local::Request::Quit).await?)
        }
        Some(Command::Reload) => println!(
            "{}",
            local::send(&cli.config, local::Request::Reload).await?
        ),
        Some(Command::Journal { enabled }) => println!(
            "{}",
            local::send(&cli.config, local::Request::View { journal: enabled }).await?
        ),
        Some(Command::Settings { command }) => match command {
            SettingsCommand::Show => println!(
                "{}",
                toml::to_string_pretty(&config_or_default(&cli.config)?)?
            ),
            SettingsCommand::Set { values } => {
                let next = values.apply(&config_or_default(&cli.config)?)?;
                next.save(&cli.config)?;
                local::notify_reload(&cli.config).await?;
                println!("Настройки сохранены.");
            }
        },
        Some(Command::Init { channels }) => {
            Config::create(&cli.config, &channels)?;
            println!("Настройки созданы. Запустите twfarmer для управления через интерфейс.");
        }
        Some(Command::Login { account }) => {
            let (mut config, profiles) = prepared(&cli.config).await?;
            let expected = account
                .map(|login| one(&profiles, Some(login)))
                .transpose()?;
            let login = accounts::login(&cli.config, &config, expected.as_ref()).await?;
            if let Some(selected) = &mut config.enabled_accounts
                && !selected.contains(&login)
            {
                selected.push(login);
                config.save(&cli.config)?;
            }
            local::notify_reload(&cli.config).await?;
        }
        Some(Command::Logout { account }) => {
            let (mut config, profiles) = prepared(&cli.config).await?;
            let profile = one(&profiles, account)?;
            accounts::remove_and_save(&cli.config, &mut config, &profile)?;
            local::notify_reload(&cli.config).await?;
            println!("Аккаунт {}: локальная сессия удалена.", profile.name);
        }
        Some(Command::Channels { channels, clear }) => {
            let mut config = config_or_default(&cli.config)?;
            if channels.is_empty() && !clear {
                println!("Общие каналы: {}", config.channels.join(", "));
            } else {
                config.channels = config::normalize_channels(&channels)?;
                config.save(&cli.config)?;
                local::notify_reload(&cli.config).await?;
                println!("Общие каналы сохранены: {}.", config.channels.join(", "));
            }
        }
        Some(Command::Accounts { command }) => match command {
            AccountsCommand::Add { channels } => {
                config::normalize_channels(&channels)?;
                let (mut config, _) = prepared(&cli.config).await?;
                let login = accounts::login(&cli.config, &config, None).await?;
                if let Some(selected) = &mut config.enabled_accounts
                    && !selected.contains(&login)
                {
                    selected.push(login.clone());
                    config.save(&cli.config)?;
                }
                if !channels.is_empty() {
                    let profiles = accounts::prepare(&cli.config, &mut config).await?;
                    let profile = one(&profiles, Some(login))?;
                    accounts::set_channels(&mut config, &profile, &channels)?;
                    config.save(&cli.config)?;
                }
                local::notify_reload(&cli.config).await?;
            }
            AccountsCommand::Select { logins, all } => {
                let (mut config, profiles) = prepared(&cli.config).await?;
                accounts::select_and_save(&cli.config, &mut config, &profiles, &logins, all)?;
                local::notify_selection(&cli.config).await?;
                println!("Выбор аккаунтов сохранён.");
            }
            AccountsCommand::List => {
                let (config, profiles) = prepared(&cli.config).await?;
                accounts::list(&profiles, &config);
            }
            AccountsCommand::Remove { login } => {
                let (mut config, profiles) = prepared(&cli.config).await?;
                let profile = one(&profiles, Some(login))?;
                accounts::remove_and_save(&cli.config, &mut config, &profile)?;
                local::notify_reload(&cli.config).await?;
                println!("Аккаунт {}: локальная сессия удалена.", profile.name);
            }
            AccountsCommand::Channels { login, channels } => {
                let (mut config, profiles) = prepared(&cli.config).await?;
                let profile = one(&profiles, Some(login))?;
                accounts::set_channels(&mut config, &profile, &channels)?;
                config.save(&cli.config)?;
                local::notify_reload(&cli.config).await?;
                println!(
                    "{}: {}.",
                    profile.name,
                    config.channels_for(&profile.config_key).join(", ")
                );
            }
        },
        Some(Command::Status {
            accounts: requested,
        }) => {
            let (config, profiles) = prepared(&cli.config).await?;
            let profiles = accounts::select(&profiles, &requested)?;
            let mut dashboard = model::Dashboard::new(
                profiles
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
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let mut failed = false;
            for profile in profiles {
                let name = profile.name.clone();
                let reporter = model::Reporter::new(name.clone(), sender.clone());
                if let Err(error) = farmer::status(profile, config.clone(), reporter.clone()).await
                {
                    failed = true;
                    reporter.account(
                        None,
                        model::AccountStatus::Error,
                        Some(format!("{error:#}")),
                    );
                    eprintln!("[{name}] {error:#}");
                }
                while let Ok(event) = receiver.try_recv() {
                    dashboard.apply(event);
                }
            }
            for (account, view) in dashboard.rows(None) {
                if view.status == model::ChannelStatus::Error {
                    failed = true;
                    eprintln!(
                        "[{}] {}: {}",
                        account.name,
                        view.name,
                        view.error.as_deref().unwrap_or("Ошибка")
                    );
                } else {
                    println!(
                        "[{}] {}: {}, {}.",
                        account.name,
                        view.name,
                        view.status.label(),
                        view.balance
                            .map_or_else(|| "—".into(), |balance| format!("{balance} баллов"))
                    );
                }
            }
            ensure!(!failed, "Не удалось получить часть балансов.");
        }
        None => app::run(cli.config, true, vec![], vec![], false, true).await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_exposes_selection_settings_clear_and_live_controls() {
        for args in [
            vec!["twfarmer", "accounts", "select", "user_one,user_two"],
            vec!["twfarmer", "accounts", "select", "--all"],
            vec!["twfarmer", "accounts", "select"],
            vec!["twfarmer", "channels", "--clear"],
            vec!["twfarmer", "settings", "show"],
            vec![
                "twfarmer",
                "settings",
                "set",
                "--max-channels",
                "1",
                "--claim-bonus",
                "false",
                "--oauth-scopes",
                "",
            ],
            vec!["twfarmer", "start", "--account", "user_one", "channel"],
            vec!["twfarmer", "stop"],
            vec!["twfarmer", "quit"],
            vec!["twfarmer", "reload"],
            vec!["twfarmer", "journal", "--enabled", "false"],
            vec!["twfarmer", "run", "--journal"],
            vec!["twfarmer", "ui", "--journal"],
            vec!["twfarmer", "init"],
            vec!["twfarmer", "daemon"],
            vec!["twfarmer", "daemon", "--account", "user_one,user_two"],
            vec!["twfarmer", "service"],
        ] {
            assert!(Cli::try_parse_from(args.clone()).is_ok(), "{args:?}");
        }
        assert!(Cli::try_parse_from(["twfarmer", "channels", "one", "--clear"]).is_err());
        assert!(Cli::try_parse_from(["twfarmer", "daemon", "new_channel"]).is_err());
        assert!(Cli::try_parse_from(["twfarmer", "daemon", "--journal"]).is_err());
    }

    #[test]
    fn cli_uses_twitch_logins_and_add_does_not_require_an_alias() {
        let cli = Cli::try_parse_from([
            "twfarmer",
            "run",
            "--account",
            "rdnlsru,other_user",
            "--no-ui",
            "channel",
        ])
        .unwrap();
        assert!(
            matches!(cli.command,Some(Command::Run{accounts,..}) if accounts==["rdnlsru","other_user"])
        );
        let cli = Cli::try_parse_from(["twfarmer", "accounts", "add"]).unwrap();
        assert!(
            matches!(cli.command,Some(Command::Accounts{command:AccountsCommand::Add{channels}}) if channels.is_empty())
        );
        let cli = Cli::try_parse_from(["twfarmer", "login"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Login { account: None })
        ));
    }
}
