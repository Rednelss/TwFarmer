use crate::{
    accounts::Profile,
    auth::SessionExpired,
    chat,
    config::Config,
    model::{AccountStatus, ChannelStatus, ChannelView, Reporter},
    twitch::{Channel, Playback, Points, Twitch},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, MissedTickBehavior, interval};
use tokio::{sync::watch, task::JoinSet};

pub type ActiveUsers = Arc<Mutex<HashMap<String, String>>>;

struct State {
    view: ChannelView,
    playback: Option<Playback>,
    failures: u32,
    retry_at: Option<Instant>,
}
impl State {
    fn new(name: String) -> Self {
        Self {
            view: ChannelView::new(name),
            playback: None,
            failures: 0,
            retry_at: None,
        }
    }
}

struct Poll {
    channel: Channel,
    points: Option<Points>,
}

fn reserve_user(users: &ActiveUsers, user_id: &str, profile: &str) -> Result<()> {
    let mut users = users.lock().map_err(|_| {
        anyhow::anyhow!(
            "{}",
            tf!(
                "Could not track active accounts",
                "Ошибка учёта активных аккаунтов"
            )
        )
    })?;
    if let Some(profile) = users.get(user_id) {
        bail!(
            "{}",
            tf!(
                "Account {profile} is already running",
                "Аккаунт {profile} уже запущен"
            )
        );
    }
    users.insert(user_id.into(), profile.into());
    Ok(())
}

pub async fn run(
    profile: Profile,
    config: Config,
    reporter: Reporter,
    users: ActiveUsers,
) -> Result<()> {
    let channels = config.channels_for(&profile.config_key).to_vec();
    ensure!(
        !channels.is_empty(),
        "{}",
        tf!(
            "Account {} has no channels. Set a shared or personal list for {}.",
            "У аккаунта {} нет каналов. Настройте общий или личный список для {}.",
            profile.name,
            profile.name
        )
    );
    let mut twitch = Twitch::connect(&config, profile.path).await?;
    reserve_user(&users, &twitch.identity.user_id, &profile.name)?;
    reporter.account(
        Some(twitch.identity.login.clone()),
        AccountStatus::Waiting,
        None,
    );
    reporter.log(
        tf!(
            "Signed in: {}. Channels: {}",
            "Вход: {}. Каналы: {}",
            twitch.identity.login,
            channels.join(", ")
        ),
        false,
    );
    let (chat_credentials, changes) = watch::channel(chat::Credentials::new(
        twitch.identity.login.clone(),
        twitch.auth.access_token.clone(),
    ));
    // Dropping the account worker also aborts its chat task.
    let mut chat_tasks = JoinSet::new();
    if config.chat_presence {
        chat_tasks.spawn(chat::run(channels.clone(), changes, reporter.clone()));
    }
    let mut states: HashMap<String, State> = channels
        .iter()
        .map(|name| (name.clone(), State::new(name.clone())))
        .collect();
    let mut selected: Vec<Channel> = vec![];
    let mut next_poll = Instant::now();
    let mut next_validation = Instant::now() + Duration::from_secs(300);
    let mut ticker = interval(Duration::from_secs(config.watch_interval_seconds));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if Instant::now() >= next_validation {
            match twitch.validate().await {
                Ok(()) => {
                    let current = chat::Credentials::new(
                        twitch.identity.login.clone(),
                        twitch.auth.access_token.clone(),
                    );
                    chat_credentials.send_if_modified(|previous| {
                        if *previous == current {
                            false
                        } else {
                            *previous = current;
                            true
                        }
                    });
                    next_validation = Instant::now() + Duration::from_secs(300);
                }
                Err(error) if error.is::<SessionExpired>() => return Err(error),
                Err(error) => {
                    reporter.log(
                        tf!(
                            "Session validation: {error:#}; retrying in a minute.",
                            "Проверка сессии: {error:#}; повтор через минуту."
                        ),
                        true,
                    );
                    next_validation = Instant::now() + Duration::from_secs(60);
                }
            }
        }
        if Instant::now() >= next_poll {
            let api = &twitch;
            let results: Vec<_> = stream::iter(channels.clone())
                .map(|name| async move {
                    let result: Result<Poll> = async {
                        let channel = api.channel(&name).await?;
                        let points = api.points(&name).await?;
                        Ok(Poll { channel, points })
                    }
                    .await;
                    (name, result)
                })
                .buffered(4)
                .collect()
                .await;
            let previous: Vec<_> = selected.iter().map(|c| c.login.clone()).collect();
            let candidates: Vec<_> = results
                .iter()
                .filter_map(|(_, result)| result.as_ref().ok())
                .filter(|poll| poll.channel.broadcast_id.is_some() && poll.points.is_some())
                .take(config.max_channels)
                .map(|poll| poll.channel.clone())
                .collect();
            let chosen: HashSet<_> = candidates.iter().map(|c| c.login.clone()).collect();
            let mut expired = false;
            for (name, result) in results {
                let state = states.get_mut(&name).context(tr!(
                    "Channel state not found",
                    "Не найдено состояние канала"
                ))?;
                match result {
                    Ok(poll) => {
                        if let Some(points) = poll.points {
                            state.view.set_balance(points.balance);
                            if chosen.contains(&name) {
                                state.view.status = if let Some(retry) = state.retry_at {
                                    ChannelStatus::Retry(retry)
                                } else if state.view.status == ChannelStatus::Watching {
                                    ChannelStatus::Watching
                                } else {
                                    ChannelStatus::Starting
                                };
                                if config.claim_bonus
                                    && let Some(claim) = points.available_claim
                                {
                                    match twitch.claim(&poll.channel, &claim).await {
                                        Ok(()) => {
                                            state.view.bonuses += 1;
                                            reporter.log(tf!("{name}: bonus claimed; the balance will update on the next poll.", "{name}: бонус получен; баланс обновится при следующем опросе."), false);
                                        }
                                        Err(error) => {
                                            expired |= error.is::<SessionExpired>();
                                            state.view.error = Some(tf!(
                                                "Claiming a bonus: {error:#}",
                                                "Получение бонуса: {error:#}"
                                            ));
                                            reporter.log(
                                                tf!(
                                                    "{name}: could not claim the bonus: {error:#}",
                                                    "{name}: бонус не получен: {error:#}"
                                                ),
                                                true,
                                            );
                                        }
                                    }
                                }
                                reporter.log(
                                    tf!(
                                        "{name}: {} points ({:+} this session).",
                                        "{name}: {} баллов ({:+} за сессию).",
                                        points.balance,
                                        state.view.gain()
                                    ),
                                    false,
                                );
                            } else {
                                state.view.status = if poll.channel.broadcast_id.is_some() {
                                    ChannelStatus::Queued
                                } else {
                                    ChannelStatus::Offline
                                };
                                state.view.error = None;
                            }
                        } else {
                            state.view.status = ChannelStatus::Disabled;
                            state.view.error = None;
                        }
                    }
                    Err(error) => {
                        expired |= error.is::<SessionExpired>();
                        state.view.status = ChannelStatus::Error;
                        state.view.error = Some(format!("{error:#}"));
                        reporter.log(format!("{name}: {error:#}"), true);
                    }
                }
                if !chosen.contains(&name) {
                    state.playback = None;
                    state.retry_at = None;
                    state.failures = 0;
                }
                reporter.channel(&state.view);
            }
            if expired {
                twitch.validate().await?;
                chat_credentials.send_replace(chat::Credentials::new(
                    twitch.identity.login.clone(),
                    twitch.auth.access_token.clone(),
                ));
                selected.clear();
                next_poll = Instant::now();
                next_validation = Instant::now() + Duration::from_secs(300);
                continue;
            }
            selected = candidates;
            let current: Vec<_> = selected.iter().map(|c| c.login.clone()).collect();
            reporter.account(
                None,
                if current.is_empty() {
                    AccountStatus::Waiting
                } else {
                    AccountStatus::Running
                },
                None,
            );
            if current != previous {
                reporter.log(
                    if current.is_empty() {
                        tr!("Waiting for online channels.", "Ожидание онлайн-каналов.").into()
                    } else {
                        tf!("Watching: {}.", "Просмотр: {}.", current.join(", "))
                    },
                    false,
                );
            }
            next_poll = Instant::now() + Duration::from_secs(config.poll_interval_seconds);
        }
        for channel in &selected {
            let state = states.get_mut(&channel.login).context(tr!(
                "Channel state not found",
                "Не найдено состояние канала"
            ))?;
            if state.retry_at.is_some_and(|at| Instant::now() < at) {
                continue;
            }
            let was_watching = state.view.status == ChannelStatus::Watching;
            let result: Result<()> = async {
                if state.playback.as_ref().is_none_or(|p| {
                    p.expires <= Instant::now()
                        || channel.broadcast_id.as_deref() != Some(p.broadcast_id.as_str())
                }) {
                    state.playback = Some(twitch.playback(channel).await?);
                }
                if let Some(playback) = &state.playback {
                    twitch.watch(channel, playback).await?;
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => {
                    let now = Instant::now();
                    if was_watching && let Some(previous) = state.view.last_success {
                        let elapsed = now.saturating_duration_since(previous);
                        if elapsed <= Duration::from_secs(config.watch_interval_seconds * 2) {
                            state.view.active_time += elapsed;
                        }
                    }
                    state.view.last_success = Some(now);
                    state.view.status = ChannelStatus::Watching;
                    state.view.error = None;
                    state.retry_at = None;
                    state.failures = 0;
                    if !was_watching {
                        reporter.log(
                            tf!(
                                "{}: watch events are being sent.",
                                "{}: события просмотра отправляются.",
                                channel.login
                            ),
                            false,
                        );
                    }
                }
                Err(error) if error.is::<SessionExpired>() => {
                    twitch.validate().await?;
                    state.playback = None;
                    state.view.status = ChannelStatus::Starting;
                    next_poll = Instant::now();
                }
                Err(error) => {
                    state.playback = None;
                    state.failures = (state.failures + 1).min(5);
                    let delay =
                        (config.watch_interval_seconds * (1 << (state.failures - 1))).min(300);
                    let at = Instant::now() + Duration::from_secs(delay);
                    state.retry_at = Some(at);
                    state.view.status = ChannelStatus::Retry(at);
                    state.view.error = Some(format!("{error:#}"));
                    reporter.log(
                        tf!(
                            "{}: {error:#}. Retrying in {delay} s.",
                            "{}: {error:#}. Повтор через {delay} с.",
                            channel.login
                        ),
                        true,
                    );
                }
            }
            reporter.channel(&state.view);
        }
    }
}

pub async fn status(profile: Profile, config: Config, reporter: Reporter) -> Result<()> {
    let twitch = Twitch::connect(&config, profile.path).await?;
    reporter.account(
        Some(twitch.identity.login.clone()),
        AccountStatus::Idle,
        None,
    );
    for name in config.channels_for(&profile.config_key) {
        let mut view = ChannelView::new(name.clone());
        match async {
            let channel = twitch.channel(name).await?;
            let points = twitch.points(name).await?;
            Ok::<_, anyhow::Error>((channel, points))
        }
        .await
        {
            Ok((channel, points)) => {
                view.status = if let Some(points) = points {
                    view.set_balance(points.balance);
                    if channel.broadcast_id.is_some() {
                        ChannelStatus::Online
                    } else {
                        ChannelStatus::Offline
                    }
                } else {
                    ChannelStatus::Disabled
                };
                reporter.log(
                    tf!(
                        "{name}: {}, balance: {}.",
                        "{name}: {}, баланс: {}.",
                        view.status.label(),
                        view.balance
                            .map_or_else(|| "-".into(), |balance| balance.to_string())
                    ),
                    false,
                );
            }
            Err(error) => {
                view.status = ChannelStatus::Error;
                view.error = Some(format!("{error:#}"));
            }
        }
        reporter.channel(&view);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_twitch_id_does_not_reserve_an_extra_profile() {
        let users = ActiveUsers::default();
        reserve_user(&users, "123", "main").unwrap();
        assert!(reserve_user(&users, "123", "copy").is_err());
        reserve_user(&users, "456", "alt").unwrap();
        assert_eq!(users.lock().unwrap().len(), 2);
        assert_eq!(users.lock().unwrap().get("123").unwrap(), "main");
    }
}
