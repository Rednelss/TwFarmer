use crate::{
    auth::SessionExpired,
    config::Config,
    twitch::{Channel, Playback, Twitch},
};
use anyhow::Result;
use futures_util::{StreamExt, stream};
use std::{collections::HashMap, time::Duration};
use tokio::time::{Instant, MissedTickBehavior, interval};

#[derive(Default)]
struct State {
    playback: Option<Playback>,
    baseline: Option<i64>,
    failures: u32,
    retry_at: Option<Instant>,
}

pub async fn run(mut twitch: Twitch, config: Config) -> Result<()> {
    println!(
        "Аккаунт: {}. Каналы: {}.",
        twitch.identity.login,
        config.channels.join(", ")
    );
    println!(
        "Одновременно: {}. Приоритет — порядок каналов. Ctrl+C — остановка.",
        config.max_channels
    );
    let mut ticker = interval(Duration::from_secs(config.watch_interval_seconds));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut next_poll = Instant::now();
    let mut next_validation = Instant::now() + Duration::from_secs(300);
    let mut selected: Vec<Channel> = vec![];
    let mut states: HashMap<String, State> = HashMap::new();
    loop {
        ticker.tick().await;
        if Instant::now() >= next_validation {
            match twitch.validate().await {
                Ok(()) => next_validation = Instant::now() + Duration::from_secs(300),
                Err(error) if error.is::<SessionExpired>() => return Err(error),
                Err(error) => {
                    eprintln!("Проверка сессии: {error:#}; повтор через минуту.");
                    next_validation = Instant::now() + Duration::from_secs(60);
                }
            }
        }
        if Instant::now() >= next_poll {
            let api = &twitch;
            let results: Vec<_> = stream::iter(config.channels.iter())
                .map(|login| async move { (login.clone(), api.channel(login).await) })
                .buffered(4)
                .collect()
                .await;
            let mut online = vec![];
            for (login, result) in results {
                match result {
                    Ok(channel) if channel.broadcast_id.is_some() => online.push(channel),
                    Ok(_) => {}
                    Err(error) if error.is::<SessionExpired>() => {
                        twitch.validate().await?;
                        next_validation = Instant::now() + Duration::from_secs(300);
                    }
                    Err(error) => eprintln!("{login}: {error:#}; проверю при следующем опросе."),
                }
            }
            let previous: Vec<_> = selected.iter().map(|c| c.login.clone()).collect();
            selected.clear();
            // A channel with disabled points must not consume a viewing slot.
            for channel in online {
                if selected.len() >= config.max_channels {
                    break;
                }
                match twitch.points(&channel.login).await {
                    Ok(Some(points)) => {
                        let state = states.entry(channel.login.clone()).or_default();
                        let baseline = *state.baseline.get_or_insert(points.balance);
                        println!(
                            "{}: {} баллов ({:+} за сессию).",
                            channel.login,
                            points.balance,
                            points.balance - baseline
                        );
                        if config.claim_bonus
                            && let Some(claim) = points.available_claim
                        {
                            match twitch.claim(&channel, &claim).await {
                                Ok(()) => println!(
                                    "{}: бонус получен; баланс обновится при следующем опросе.",
                                    channel.login
                                ),
                                Err(error) if error.is::<SessionExpired>() => {
                                    twitch.validate().await?;
                                }
                                Err(error) => {
                                    eprintln!("{}: бонус не получен: {error:#}", channel.login)
                                }
                            }
                        }
                        selected.push(channel);
                    }
                    Ok(None) => println!("{}: баллы канала отключены.", channel.login),
                    Err(error) if error.is::<SessionExpired>() => {
                        twitch.validate().await?;
                    }
                    Err(error) => eprintln!("{}: {error:#}", channel.login),
                }
            }
            let current: Vec<_> = selected.iter().map(|c| c.login.clone()).collect();
            if current.is_empty() {
                println!(
                    "Ожидаю онлайн-каналы с доступными баллами. Следующая проверка через {} с.",
                    config.poll_interval_seconds
                );
            } else if current != previous {
                println!("Просмотр: {}.", current.join(", "));
            }
            // Keep balances across channel switches; discard signed playback URLs.
            for (login, state) in &mut states {
                if !current.contains(login) {
                    state.playback = None;
                    state.retry_at = None;
                    state.failures = 0;
                }
            }
            next_poll = Instant::now() + Duration::from_secs(config.poll_interval_seconds);
        }
        for channel in &selected {
            let state = states.entry(channel.login.clone()).or_default();
            if state.retry_at.is_some_and(|time| Instant::now() < time) {
                continue;
            }
            let result: Result<()> = async {
                let reload = state.playback.as_ref().is_none_or(|p| {
                    p.expires <= Instant::now()
                        || channel.broadcast_id.as_deref() != Some(p.broadcast_id.as_str())
                });
                if reload {
                    state.playback = Some(twitch.playback(channel).await?);
                }
                if let Some(playback) = state.playback.as_ref() {
                    twitch.watch(channel, playback).await?;
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => {
                    state.failures = 0;
                    state.retry_at = None;
                }
                Err(error) if error.is::<SessionExpired>() => {
                    twitch.validate().await?;
                    state.playback = None;
                }
                Err(error) => {
                    state.playback = None;
                    state.failures = (state.failures + 1).min(5);
                    let delay =
                        (config.watch_interval_seconds * (1 << (state.failures - 1))).min(300);
                    state.retry_at = Some(Instant::now() + Duration::from_secs(delay));
                    eprintln!(
                        "{}: {error:#}. Повтор просмотра через {delay} с.",
                        channel.login
                    );
                }
            }
        }
    }
}
