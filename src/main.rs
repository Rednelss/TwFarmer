mod auth;
mod config;
mod farmer;
mod hls;
mod tracking;
mod twitch;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use config::Config;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "twfarmer",
    version,
    about = "Twitch: просмотр выбранных каналов и сбор бонусных баллов"
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
    #[command(about = "Создать настройки; без аргументов запросит каналы")]
    Init {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = "Войти через код устройства на сайте Twitch")]
    Login,
    #[command(about = "Запустить фарм; аргументы заменяют список каналов на один запуск")]
    Run {
        #[arg(value_name = "CHANNELS")]
        channels: Vec<String>,
    },
    #[command(about = "Показать онлайн-статус и баланс каналов")]
    Status,
    #[command(about = "Удалить сохранённую локальную сессию")]
    Logout,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    tokio::select! {
        result = execute(cli) => if let Err(error) = result {
            eprintln!("Ошибка: {error:#}");
            std::process::exit(1);
        },
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                eprintln!("Не удалось установить обработчик Ctrl+C: {error}");
                std::process::exit(1);
            }
            println!("\nОстановлено.");
        }
    }
}

async fn execute(cli: Cli) -> Result<()> {
    let auth_path = config::auth_path(&cli.config);
    match cli.command {
        Some(Command::Init { mut channels }) => {
            if channels.is_empty() {
                channels = prompt_channels()?;
            }
            Config::create(&cli.config, &channels)?;
            println!(
                "Настройки созданы: {}. Далее: twfarmer login, затем twfarmer run.",
                cli.config.display()
            );
        }
        Some(Command::Logout) => match std::fs::remove_file(&auth_path) {
            Ok(()) => println!("Сохранённая сессия удалена."),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                println!("Сохранённой сессии нет.")
            }
            Err(error) => return Err(error).context("Не удалось удалить сессию"),
        },
        Some(Command::Login) => {
            let config = if cli.config.exists() {
                Config::read(&cli.config)?
            } else {
                Config::default()
            };
            let identity = auth::login(&twitch::http_client()?, &config, &auth_path).await?;
            println!("Вход выполнен: {}. Сессия сохранена.", identity.login);
        }
        Some(Command::Status) => {
            let config = Config::read(&cli.config)?;
            ensure!(!config.channels.is_empty(), "Список каналов пуст");
            let twitch = twitch::Twitch::connect(&config, auth_path).await?;
            println!("Аккаунт: {}", twitch.identity.login);
            let mut failed = false;
            for login in &config.channels {
                match async {
                    let channel = twitch.channel(login).await?;
                    let state = if channel.broadcast_id.is_some() {
                        "онлайн"
                    } else {
                        "офлайн"
                    };
                    let balance = match twitch.points(login).await? {
                        Some(points) => format!("{} баллов", points.balance),
                        None => "баллы отключены".into(),
                    };
                    Ok::<_, anyhow::Error>(format!("{login}: {state}, {balance}"))
                }
                .await
                {
                    Ok(line) => println!("{line}"),
                    Err(error) => {
                        failed = true;
                        eprintln!("{login}: {error:#}");
                    }
                }
            }
            ensure!(!failed, "Не удалось получить статус всех каналов");
        }
        command => {
            let channels = match command {
                Some(Command::Run { channels }) => channels,
                None => vec![],
                _ => unreachable!(),
            };
            if !cli.config.exists() {
                let initial = if channels.is_empty() {
                    prompt_channels()?
                } else {
                    channels.clone()
                };
                Config::create(&cli.config, &initial)?;
                println!("Настройки созданы: {}", cli.config.display());
            }
            let mut config = Config::read(&cli.config)?;
            if !channels.is_empty() {
                config.channels = config::normalize_channels(&channels)?;
            }
            ensure!(
                !config.channels.is_empty(),
                "Укажите каналы в конфигурации или в twfarmer run CHANNELS"
            );
            if !auth_path.exists() {
                auth::login(&twitch::http_client()?, &config, &auth_path).await?;
            }
            let twitch = twitch::Twitch::connect(&config, auth_path).await?;
            farmer::run(twitch, config).await?;
        }
    }
    Ok(())
}

fn prompt_channels() -> Result<Vec<String>> {
    ensure!(
        io::stdin().is_terminal(),
        "Укажите каналы аргументами: twfarmer init channel1 channel2"
    );
    print!("Каналы через пробел или запятую: ");
    io::stdout().flush()?;
    let mut text = String::new();
    io::stdin().read_line(&mut text)?;
    config::normalize_channels(&[text])
}
