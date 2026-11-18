#[macro_use]
mod i18n;
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
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use config::Config;
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "twfarmer",
    version,
    about = tr!("Twitch: channel points farming and account management in the terminal", "Twitch: фарм баллов и управление аккаунтами в консоли")
)]
struct Cli {
    #[arg(long = "lang", global = true, value_enum, help = tr!("Language for this run: en or ru", "Язык текущего запуска: en или ru"))]
    lang: Option<i18n::Language>,
    #[arg(
        long,
        global = true,
        default_value = "twfarmer.toml",
        help = tr!("Settings file path", "Путь к настройкам")
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    #[command(about = tr!("Show or save the language: en or ru", "Показать или сохранить язык: en или ru"))]
    Language {
        #[arg(value_enum)]
        language: Option<i18n::Language>,
    },
    #[command(about = tr!("Open the management interface", "Открыть интерфейс управления"))]
    Ui {
        #[arg(long)]
        journal: bool,
    },
    #[command(about = tr!("Start farming in the running program", "Запустить фарм в уже открытой программе"))]
    Start {
        #[arg(long = "account", value_delimiter = ',')]
        accounts: Vec<String>,
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = tr!("Stop farming in the running program", "Остановить фарм в запущенной программе"))]
    Stop,
    #[command(about = tr!("Close the running program", "Закрыть запущенную программу"))]
    Quit,
    #[command(about = tr!("Reload settings in the running program", "Перечитать настройки в запущенной программе"))]
    Reload,
    #[command(about = tr!("Show the log or restore the table in the running program", "Показать журнал или вернуть таблицу в запущенной программе"))]
    Journal {
        #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
        enabled: bool,
    },
    #[command(about = tr!("View and change farming settings", "Просмотр и изменение настроек фарма"))]
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    #[command(about = tr!("Create settings with shared channels", "Создать настройки с общими каналами"))]
    Init {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = tr!("Add an account; its name is determined by Twitch", "Добавить аккаунт; имя определяется по Twitch автоматически"))]
    Login {
        #[arg(long, help = tr!("Twitch login of an existing account to renew", "Логин Twitch для обновления существующего входа"))]
        account: Option<String>,
    },
    #[command(about = tr!("Start farming; opens the interface in an interactive terminal", "Запустить фарм; в терминале открывается интерфейс управления"))]
    Run {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
        #[arg(
            long = "account",
            value_delimiter = ',',
            help = tr!("Twitch logins separated by commas", "Логины Twitch через запятую")
        )]
        accounts: Vec<String>,
        #[arg(long, help = tr!("Plain log without the interface", "Обычный журнал без интерфейса"))]
        no_ui: bool,
        #[arg(long, help = tr!("Open the log instead of the table", "Открыть журнал вместо таблицы"))]
        journal: bool,
    },
    #[command(
        alias = "service",
        about = tr!("Daemon startup: existing configuration and saved sessions only", "Служебный запуск: только готовая конфигурация и сохранённые сессии")
    )]
    Daemon {
        #[arg(
            long = "account",
            value_delimiter = ',',
            help = tr!("Saved Twitch logins; otherwise use the configured selection", "Сохранённые логины Twitch; иначе выбор из конфигурации")
        )]
        accounts: Vec<String>,
    },
    #[command(about = tr!("Show account balances without farming", "Показать балансы аккаунтов без фарма"))]
    Status {
        #[arg(long = "account", value_delimiter = ',')]
        accounts: Vec<String>,
    },
    #[command(about = tr!("Remove a local account session", "Удалить локальную сессию аккаунта"))]
    Logout {
        #[arg(long, help = tr!("Twitch login; optional if there is only one account", "Логин Twitch; можно опустить, если аккаунт один"))]
        account: Option<String>,
    },
    #[command(about = tr!("Shared channels; show the list when no channels are provided", "Общие каналы; без аргументов показать список"))]
    Channels {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
        #[arg(long, conflicts_with = "channels", help = tr!("Clear the shared list", "Очистить общий список"))]
        clear: bool,
    },
    #[command(about = tr!("Manage accounts by Twitch login", "Управление аккаунтами по логину Twitch"))]
    Accounts {
        #[command(subcommand)]
        command: AccountsCommand,
    },
}
#[derive(Subcommand)]
enum AccountsCommand {
    #[command(about = tr!("Sign in; optional arguments set personal channels", "Войти в аккаунт; необязательные аргументы задают личные каналы"))]
    Add {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = tr!("Show accounts and their channel lists", "Показать аккаунты и списки каналов"))]
    List,
    #[command(about = tr!("Save the account selection; no logins clears the selection", "Сохранить выбор аккаунтов; без логинов снять выбор"))]
    Select {
        #[arg(value_name = "TWITCH_LOGINS", value_delimiter = ',')]
        logins: Vec<String>,
        #[arg(long, conflicts_with = "logins", help = tr!("Use all accounts", "Использовать все аккаунты"))]
        all: bool,
    },
    #[command(about = tr!("Remove a local account session", "Удалить локальную сессию аккаунта"))]
    Remove {
        #[arg(value_name = "TWITCH_LOGIN")]
        login: String,
    },
    #[command(about = tr!("Personal account channels; omit channels to use the shared list", "Личные каналы аккаунта; без каналов вернуть общий список"))]
    Channels {
        #[arg(value_name = "TWITCH_LOGIN")]
        login: String,
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
}

#[derive(Subcommand)]
enum SettingsCommand {
    #[command(about = tr!("Show settings", "Показать настройки"))]
    Show,
    #[command(about = tr!("Change the specified settings", "Изменить указанные настройки"))]
    Set {
        #[command(flatten)]
        values: config::SettingsPatch,
    },
}

#[tokio::main]
async fn main() {
    i18n::initialize(&std::env::args_os().collect::<Vec<_>>());
    let mut command = localized_command(Cli::command());
    let matches = command
        .try_get_matches_from_mut(std::env::args_os())
        .unwrap_or_else(|error| cli_error(error));
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let result = tokio::select! {
        result=execute(cli)=>Some(result),
        result=shutdown_signal()=>match result {Ok(())=>None,Err(error)=>Some(Err(error.into()))},
    };
    match result {
        Some(Err(error)) => {
            eprintln!("{}", tf!("Error: {error:#}", "Ошибка: {error:#}"));
            std::process::exit(1);
        }
        None => println!("{}", tf!("\nStopped.", "\nОстановлено.")),
        Some(Ok(())) => {}
    }
}

fn cli_error(error: clap::Error) -> ! {
    if i18n::current() != i18n::Language::Ru || !error.use_stderr() {
        error.exit();
    }
    use clap::error::{ContextKind, ErrorKind};
    let reason = match error.kind() {
        ErrorKind::UnknownArgument => "Неизвестный аргумент",
        ErrorKind::InvalidSubcommand => "Неизвестная команда",
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => "Недопустимое значение параметра",
        ErrorKind::MissingRequiredArgument => "Не указаны обязательные параметры",
        ErrorKind::MissingSubcommand => "Не указана команда",
        ErrorKind::ArgumentConflict => "Несовместимые параметры",
        ErrorKind::TooManyValues | ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
            "Неверное количество значений"
        }
        ErrorKind::InvalidUtf8 => "Аргумент содержит некорректный UTF-8",
        _ => "Неверные аргументы команды",
    };
    eprintln!("Ошибка: {reason}");
    for (kind, label) in [
        (ContextKind::InvalidArg, "Параметр"),
        (ContextKind::InvalidSubcommand, "Команда"),
        (ContextKind::InvalidValue, "Значение"),
        (ContextKind::PriorArg, "Другой параметр"),
        (ContextKind::ValidValue, "Допустимые значения"),
    ] {
        if let Some(value) = error.get(kind) {
            eprintln!("{label}: {value}");
        }
    }
    if let Some(usage) = error.get(ContextKind::Usage) {
        eprintln!(
            "\n{}",
            usage.to_string().replacen("Usage:", "Использование:", 1)
        );
    }
    eprintln!("\nДля справки используйте --help.");
    std::process::exit(error.exit_code());
}

fn localized_command(mut command: clap::Command) -> clap::Command {
    if command.get_name() == "help" {
        command = command.about(tr!(
            "Print help for a command",
            "Показать справку по команде"
        ));
    }
    command.build();
    let arguments = command
        .get_arguments()
        .map(|arg| (arg.get_id().clone(), arg.is_positional()))
        .collect::<Vec<_>>();
    for (id, positional) in arguments {
        command = command.mut_arg(id, |arg| {
            let arg =
                if i18n::current() == i18n::Language::Ru && !arg.get_default_values().is_empty() {
                    let default = arg
                        .get_default_values()
                        .iter()
                        .map(|value| value.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
                    arg.help(format!("{help} [по умолчанию: {default}]"))
                        .hide_default_value(true)
                } else {
                    arg
                };
            arg.hide_possible_values(i18n::current() == i18n::Language::Ru)
                .help_heading(if positional {
                    tr!("Arguments", "Аргументы")
                } else {
                    tr!("Options", "Параметры")
                })
        });
    }
    if command.get_arguments().any(|arg| arg.get_id() == "help") {
        command = command.mut_arg("help", |arg| {
            arg.help(tr!("Print help", "Показать справку"))
        });
    }
    if command.get_arguments().any(|arg| arg.get_id() == "version") {
        command = command.mut_arg("version", |arg| {
            arg.help(tr!("Print version", "Показать версию"))
        });
    }
    command
        .subcommand_help_heading(tr!("Commands", "Команды"))
        .help_template(tr!(
            "{about-with-newline}\nUsage: {usage}\n\n{all-args}{after-help}",
            "{about-with-newline}\nИспользование: {usage}\n\n{all-args}{after-help}"
        ))
        .mut_subcommands(localized_command)
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
            .ok_or_else(|| anyhow::anyhow!("{}", tf!("Account not found", "Аккаунт не найден")));
    }
    ensure!(
        profiles.len() == 1,
        "{}",
        tf!(
            "Specify a Twitch login with --account or select an account in the interface.",
            "Укажите логин Twitch через --account или выберите аккаунт в интерфейсе."
        )
    );
    Ok(profiles[0].clone())
}

async fn execute(cli: Cli) -> Result<()> {
    if let Some(language) = cli.lang {
        i18n::set_override(Some(language));
    }
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    match cli.command {
        Some(Command::Language { language }) => {
            let mut config = config_or_default(&cli.config)?;
            if let Some(language) = language {
                config.language = language;
                config.save(&cli.config)?;
                i18n::set_override(None);
                i18n::apply_config(language);
                local::notify_reload(&cli.config).await?;
            }
            println!(
                "{}",
                tf!(
                    "Language: {} ({})",
                    "Язык: {} ({})",
                    config.language.name(),
                    config.language
                )
            );
        }
        Some(Command::Ui { journal }) => {
            ensure!(
                terminal,
                "{}",
                tf!(
                    "The interface requires an interactive terminal.",
                    "Интерфейсу нужен интерактивный терминал."
                )
            );
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
                i18n::apply_config(next.language);
                local::notify_reload(&cli.config).await?;
                println!("{}", tf!("Settings saved.", "Настройки сохранены."));
            }
        },
        Some(Command::Init { channels }) => {
            Config::create(&cli.config, &channels)?;
            println!(
                "{}",
                tf!(
                    "Settings created. Run twfarmer to manage them in the interface.",
                    "Настройки созданы. Запустите twfarmer для управления через интерфейс."
                )
            );
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
            println!(
                "{}",
                tf!(
                    "Account {}: local session removed.",
                    "Аккаунт {}: локальная сессия удалена.",
                    profile.name
                )
            );
        }
        Some(Command::Channels { channels, clear }) => {
            let mut config = config_or_default(&cli.config)?;
            if channels.is_empty() && !clear {
                println!(
                    "{}",
                    tf!(
                        "Shared channels: {}",
                        "Общие каналы: {}",
                        config.channels.join(", ")
                    )
                );
            } else {
                config.channels = config::normalize_channels(&channels)?;
                config.save(&cli.config)?;
                local::notify_reload(&cli.config).await?;
                println!(
                    "{}",
                    tf!(
                        "Shared channels saved: {}.",
                        "Общие каналы сохранены: {}.",
                        config.channels.join(", ")
                    )
                );
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
                println!(
                    "{}",
                    tf!("Account selection saved.", "Выбор аккаунтов сохранён.")
                );
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
                println!(
                    "{}",
                    tf!(
                        "Account {}: local session removed.",
                        "Аккаунт {}: локальная сессия удалена.",
                        profile.name
                    )
                );
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
                        view.error.as_deref().unwrap_or(tr!("Error", "Ошибка"))
                    );
                } else {
                    println!(
                        "[{}] {}: {}, {}.",
                        account.name,
                        view.name,
                        view.status.label(),
                        view.balance.map_or_else(
                            || "-".into(),
                            |balance| tf!("{balance} points", "{balance} баллов")
                        )
                    );
                }
            }
            ensure!(
                !failed,
                "{}",
                tf!(
                    "Could not retrieve some balances.",
                    "Не удалось получить часть балансов."
                )
            );
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
