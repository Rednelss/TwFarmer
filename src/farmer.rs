use crate::{
    accounts::Profile,
    auth::SessionExpired,
    chat,
    config::{Config, FarmingPriority},
    drops::{Drop, DropStatus},
    model::{AccountStatus, ChannelStatus, ChannelView, FarmRole, Reporter},
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

async fn claim_ready(twitch: &Twitch, drops: &mut [Drop], reporter: &Reporter) -> Result<()> {
    let mut failure = None;
    for drop in drops
        .iter_mut()
        .filter(|d| d.status == DropStatus::Claimable)
    {
        match twitch.claim_drop(drop).await {
            Ok(()) => {
                reporter.log(
                    tf!(
                        "Drop claimed: {} / {}.",
                        "Drop получен: {} / {}.",
                        drop.campaign,
                        drop.reward
                    ),
                    false,
                );
                drop.status = DropStatus::Claimed;
            }
            Err(error) => {
                reporter.log(
                    tf!(
                        "Claiming {}: {error:#}",
                        "Получение {}: {error:#}",
                        drop.reward
                    ),
                    true,
                );
                if failure.is_none() || error.is::<SessionExpired>() {
                    failure = Some(error);
                }
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

struct Poll {
    channel: Channel,
    points: Option<Points>,
    drops: Result<Vec<Drop>>,
}

fn has_target(poll: &Poll, configured: &[String], catalog: &[crate::drops::CatalogEntry]) -> bool {
    poll.drops.as_ref().is_ok_and(|drops| {
        drops.iter().any(|drop| {
            drop.pending()
                && (configured.contains(&poll.channel.login)
                    || catalog.iter().any(|entry| {
                        entry.drop.pending()
                            && entry.drop.campaign_id == drop.campaign_id
                            && entry.drop.group_id == drop.group_id
                    }))
        })
    })
}

struct Selection {
    channels: Vec<Channel>,
    drop_channel: Option<String>,
}

fn select_channels(
    results: &[(String, Result<Poll>)],
    configured: &[String],
    catalog: &[crate::drops::CatalogEntry],
    blocked: &HashSet<String>,
    max: usize,
    priority: FarmingPriority,
) -> Selection {
    let online: Vec<_> = results
        .iter()
        .filter_map(|(_, r)| r.as_ref().ok())
        .filter(|p| p.channel.broadcast_id.is_some() && !blocked.contains(&p.channel.login))
        .collect();
    let mut selection = Selection {
        channels: vec![],
        drop_channel: None,
    };
    let drop = online.iter().find(|p| has_target(p, configured, catalog));
    if priority == FarmingPriority::Drops
        && max > 0
        && let Some(poll) = drop
    {
        selection.channels.push(poll.channel.clone());
        selection.drop_channel = Some(poll.channel.login.clone());
    }
    for poll in &online {
        if selection.channels.len() >= max {
            break;
        }
        if configured.contains(&poll.channel.login)
            && poll.points.is_some()
            && !selection
                .channels
                .iter()
                .any(|c| c.login == poll.channel.login)
        {
            selection.channels.push(poll.channel.clone());
        }
    }
    if selection.drop_channel.is_none() {
        if let Some(poll) = online.iter().find(|p| {
            selection
                .channels
                .iter()
                .any(|c| c.login == p.channel.login)
                && has_target(p, configured, catalog)
        }) {
            selection.drop_channel = Some(poll.channel.login.clone());
        } else if selection.channels.len() < max
            && let Some(poll) = drop
        {
            selection.channels.push(poll.channel.clone());
            selection.drop_channel = Some(poll.channel.login.clone());
        }
    }
    selection
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
        !channels.is_empty() || config.automatic_drops(),
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
    reporter.log(
        tf!(
            "Farming priority: {}. Video slots: {}; at most one for Drops.",
            "Приоритет фарма: {}. Видеослотов: {}; не более одного для Drops.",
            config.farming_priority.label(),
            config.max_channels
        ),
        false,
    );
    reporter.log(
        if config.automatic_drops() {
            tr!("Automatic Drops search is enabled. One Drops stream per account, when the selected priority leaves a slot.", "Автопоиск Drops включён. Один поток Drops на аккаунт, если выбранный приоритет оставляет слот.")
        } else if config.farm_drops {
            tr!("Drops are limited to configured channels. Automatic search is off; enable it with settings set --auto-drops true.", "Drops только на настроенных каналах. Автопоиск выключен; включить: settings set --auto-drops true.")
        } else {
            tr!("Drops farming is disabled; enable it with settings set --farm-drops true.", "Фарм Drops выключен; включить: settings set --farm-drops true.")
        }, false,
    );
    let (chat_credentials, changes) = watch::channel(chat::Credentials::new(
        twitch.identity.login.clone(),
        twitch.auth.access_token.clone(),
    ));
    // Dropping the account worker also aborts its chat task.
    let mut chat_tasks = JoinSet::new();
    if config.chat_presence && !channels.is_empty() {
        chat_tasks.spawn(chat::run(channels.clone(), changes, reporter.clone()));
    }
    let mut states: HashMap<String, State> = channels
        .iter()
        .map(|name| {
            let mut state = State::new(name.clone());
            state.view.drops_enabled = config.farm_drops;
            (name.clone(), state)
        })
        .collect();
    let mut selected: Vec<Channel> = vec![];
    let mut drop_channel: Option<String> = None;
    let mut selection_reported = false;
    let mut catalog = vec![];
    let mut catalog_error = None;
    let mut next_catalog = Instant::now();
    let mut auto_chat_tasks = JoinSet::new();
    let mut chat_channels = vec![];
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
            let mut catalog_changed = false;
            let api = &twitch;
            let mut polled = channels.clone();
            for channel in &selected {
                if !polled.contains(&channel.login) {
                    polled.push(channel.login.clone());
                }
            }
            let mut results: Vec<_> = stream::iter(polled)
                .map(|name| async move {
                    let result: Result<Poll> = async {
                        let channel = api.channel(&name).await?;
                        let points = api.points(&name).await?;
                        let drops = if config.farm_drops && channel.broadcast_id.is_some() {
                            api.drops(&channel).await
                        } else {
                            Ok(vec![])
                        };
                        Ok(Poll {
                            channel,
                            points,
                            drops,
                        })
                    }
                    .await;
                    (name, result)
                })
                .buffered(4)
                .collect()
                .await;
            let previous: Vec<_> = selected.iter().map(|c| c.login.clone()).collect();
            let previous_drop = drop_channel.clone();
            let points_slots = results
                .iter()
                .filter(|(name, result)| {
                    channels.contains(name)
                        && states.get(name).is_none_or(|state| state.failures < 3)
                        && result
                            .as_ref()
                            .is_ok_and(|p| p.channel.broadcast_id.is_some() && p.points.is_some())
                })
                .count();
            if config.automatic_drops()
                && Instant::now() >= next_catalog
                && (config.farming_priority == FarmingPriority::Drops
                    || points_slots < config.max_channels)
            {
                match twitch.available_drops().await {
                    Ok(mut entries) => {
                        if config.claim_drops {
                            for entry in entries
                                .iter_mut()
                                .filter(|e| e.drop.status == DropStatus::Claimable)
                            {
                                if let Err(error) = claim_ready(
                                    &twitch,
                                    std::slice::from_mut(&mut entry.drop),
                                    &reporter,
                                )
                                .await
                                    && error.is::<SessionExpired>()
                                {
                                    twitch.validate().await?;
                                }
                            }
                        }
                        reporter.log(
                            tf!(
                                "Eligible Drops rewards: {}.",
                                "Можно фармить наград Drops: {}.",
                                entries.iter().filter(|e| e.drop.pending()).count()
                            ),
                            false,
                        );
                        catalog = entries;
                        catalog_error = None;
                        catalog_changed = true;
                        next_catalog = Instant::now() + Duration::from_secs(300);
                    }
                    Err(error) => {
                        if error.is::<SessionExpired>() {
                            twitch.validate().await?;
                        }
                        catalog_error = Some(format!("{error:#}"));
                        catalog_changed = true;
                        reporter.log(
                            tf!("Drops campaigns: {error:#}", "Кампании Drops: {error:#}"),
                            true,
                        );
                        next_catalog = Instant::now() + Duration::from_secs(60);
                    }
                }
            }
            for (_, result) in &results {
                if let Ok(poll) = result
                    && let Ok(drops) = &poll.drops
                {
                    for drop in drops {
                        if let Some(entry) = catalog.iter_mut().find(|e| {
                            e.drop.campaign_id == drop.campaign_id
                                && e.drop.group_id == drop.group_id
                        }) {
                            entry.drop = drop.clone();
                        }
                    }
                }
            }
            let blocked = states
                .iter()
                .filter(|(_, s)| s.failures >= 3)
                .map(|(name, _)| name.clone())
                .collect();
            let mut selection = select_channels(
                &results,
                &channels,
                &catalog,
                &blocked,
                config.max_channels,
                config.farming_priority,
            );
            if config.automatic_drops()
                && selection.drop_channel.is_none()
                && (selection.channels.len() < config.max_channels
                    || config.farming_priority == FarmingPriority::Drops)
            {
                let excluded = results.iter().map(|(name, _)| name.clone()).collect();
                match twitch.find_drop_channel(&catalog, &excluded).await {
                    Ok(Some((channel, drops))) => {
                        let name = channel.login.clone();
                        let points = twitch.points(&name).await.unwrap_or(None);
                        let mut state = State::new(name.clone());
                        state.view.automatic = !channels.contains(&name);
                        state.view.drops_enabled = true;
                        reporter.channel(&state.view);
                        states.insert(name.clone(), state);
                        reporter.log(
                            tf!(
                                "Auto Drops: watching {name}.",
                                "Авто-Drops: просмотр {name}."
                            ),
                            false,
                        );
                        results.push((
                            name,
                            Ok(Poll {
                                channel,
                                points,
                                drops: Ok(drops),
                            }),
                        ));
                        selection = select_channels(
                            &results,
                            &channels,
                            &catalog,
                            &blocked,
                            config.max_channels,
                            config.farming_priority,
                        );
                    }
                    Ok(None) => {
                        let message = tr!(
                            "No online channel for eligible Drops; checking again in a minute.",
                            "Нет онлайн-канала для доступных Drops; повтор через минуту."
                        );
                        reporter.log(message, false);
                    }
                    Err(error) => {
                        reporter.log(
                            tf!(
                                "Automatic Drops search: {error:#}",
                                "Автопоиск Drops: {error:#}"
                            ),
                            true,
                        );
                        if error.is::<SessionExpired>() {
                            twitch.validate().await?;
                        }
                    }
                }
            }
            let chosen: HashSet<_> = selection.channels.iter().map(|c| c.login.clone()).collect();
            let mut expired = false;
            for (name, result) in results {
                let state = states.get_mut(&name).context(tr!(
                    "Channel state not found",
                    "Не найдено состояние канала"
                ))?;
                state.view.farm_role = FarmRole::None;
                match result {
                    Ok(poll) => {
                        if chosen.contains(&name) {
                            state.view.farm_role = match (
                                selection.drop_channel.as_deref() == Some(name.as_str()),
                                poll.points.is_some(),
                            ) {
                                (true, true) => FarmRole::DropsAndPoints,
                                (true, false) => FarmRole::Drops,
                                _ => FarmRole::Points,
                            };
                        }
                        match poll.drops {
                            Ok(mut drops) => {
                                let recovering = state.view.drops_error.is_some();
                                state.view.drops_error = None;
                                if config.claim_drops
                                    && let Err(error) =
                                        claim_ready(&twitch, &mut drops, &reporter).await
                                {
                                    expired |= error.is::<SessionExpired>();
                                    state.view.drops_error = Some(format!("{error:#}"));
                                }
                                for drop in &drops {
                                    if let Some(entry) = catalog.iter_mut().find(|e| {
                                        e.drop.campaign_id == drop.campaign_id
                                            && e.drop.group_id == drop.group_id
                                    }) && entry.drop != *drop
                                    {
                                        entry.drop = drop.clone();
                                        catalog_changed = true;
                                    }
                                    if recovering || !state.view.drops.contains(drop) {
                                        reporter.log(format!("{name}: {}", drop.detail()), false);
                                    }
                                }
                                state.view.drops = drops;
                                state.view.drops_updated = Some(Instant::now());
                            }
                            Err(error) => {
                                expired |= error.is::<SessionExpired>();
                                state.view.drops_error = Some(format!("{error:#}"));
                                reporter.log(
                                    tf!(
                                        "{name}: Drops progress: {error:#}",
                                        "{name}: прогресс Drops: {error:#}"
                                    ),
                                    true,
                                );
                            }
                        }
                        if let Some(points) = poll.points {
                            let changed = state.view.balance != Some(points.balance)
                                || !previous.contains(&name);
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
                                if changed {
                                    reporter.log(
                                        tf!(
                                            "{name}: {} points ({:+} this session).",
                                            "{name}: {} баллов ({:+} за сессию).",
                                            points.balance,
                                            state.view.gain()
                                        ),
                                        false,
                                    );
                                }
                            } else {
                                state.view.status = if poll.channel.broadcast_id.is_some() {
                                    ChannelStatus::Queued
                                } else {
                                    ChannelStatus::Offline
                                };
                                state.view.error = None;
                            }
                        } else {
                            state.view.balance = None;
                            state.view.status = if poll.channel.broadcast_id.is_none() {
                                ChannelStatus::Offline
                            } else if chosen.contains(&name) {
                                if let Some(retry) = state.retry_at {
                                    ChannelStatus::Retry(retry)
                                } else if state.view.status == ChannelStatus::Watching {
                                    ChannelStatus::Watching
                                } else {
                                    ChannelStatus::Starting
                                }
                            } else if state.view.drops.iter().any(Drop::pending) {
                                ChannelStatus::Queued
                            } else {
                                ChannelStatus::Disabled
                            };
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
            selected = selection.channels;
            drop_channel = selection.drop_channel;
            let current_names: HashSet<_> = selected.iter().map(|c| c.login.clone()).collect();
            let removed: Vec<_> = states
                .iter()
                .filter(|(name, state)| state.view.automatic && !current_names.contains(*name))
                .map(|(name, _)| name.clone())
                .collect();
            for name in removed {
                states.remove(&name);
                reporter.remove_channel(&name);
            }
            let wanted_chat: Vec<_> = selected
                .iter()
                .filter(|c| !channels.contains(&c.login))
                .map(|c| c.login.clone())
                .collect();
            if config.chat_presence && wanted_chat != chat_channels {
                auto_chat_tasks.abort_all();
                while auto_chat_tasks.join_next().await.is_some() {}
                if !wanted_chat.is_empty() {
                    auto_chat_tasks.spawn(chat::run(
                        wanted_chat.clone(),
                        chat_credentials.subscribe(),
                        reporter.clone(),
                    ));
                }
                chat_channels = wanted_chat;
            }
            if config.automatic_drops() && catalog_changed {
                reporter.campaigns(&catalog, catalog_error.clone());
            }
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
            if current != previous || drop_channel != previous_drop || !selection_reported {
                reporter.log(
                    if current.is_empty() {
                        tr!("Waiting for online channels.", "Ожидание онлайн-каналов.").into()
                    } else {
                        let names = selected
                            .iter()
                            .map(|c| {
                                format!("{} ({})", c.login, states[&c.login].view.farm_role.label())
                            })
                            .collect::<Vec<_>>();
                        tf!("Watching: {}.", "Просмотр: {}.", names.join(", "))
                    },
                    false,
                );
                selection_reported = true;
            }
            next_poll = Instant::now()
                + Duration::from_secs(if config.farm_drops {
                    config.poll_interval_seconds.min(60)
                } else {
                    config.poll_interval_seconds
                });
        }
        // Keep the Drops heartbeat after points-only streams in each watch cycle.
        let mut watch_order: Vec<_> = selected.iter().collect();
        watch_order.sort_by_key(|channel| drop_channel.as_deref() == Some(channel.login.as_str()));
        for channel in watch_order {
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
                    twitch
                        .watch(
                            channel,
                            playback,
                            drop_channel.as_deref() == Some(channel.login.as_str()),
                        )
                        .await?;
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

pub async fn campaigns(
    profile: Profile,
    config: &Config,
) -> Result<Vec<crate::drops::CatalogEntry>> {
    Twitch::connect(config, profile.path)
        .await?
        .available_drops()
        .await
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
        view.drops_enabled = config.farm_drops;
        match async {
            let channel = twitch.channel(name).await?;
            let points = twitch.points(name).await?;
            Ok::<_, anyhow::Error>((channel, points))
        }
        .await
        {
            Ok((channel, points)) => {
                if config.farm_drops && channel.broadcast_id.is_some() {
                    match twitch.drops(&channel).await {
                        Ok(drops) => {
                            for drop in &drops {
                                reporter.log(format!("{name}: {}", drop.detail()), false);
                            }
                            view.drops = drops;
                            view.drops_updated = Some(Instant::now());
                        }
                        Err(error) => {
                            view.drops_error = Some(format!("{error:#}"));
                            reporter.log(
                                tf!(
                                    "{name}: Drops progress: {error:#}",
                                    "{name}: прогресс Drops: {error:#}"
                                ),
                                true,
                            );
                        }
                    }
                }
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
    fn poll(name: &str, online: bool, points: bool, drops: Vec<Drop>) -> (String, Result<Poll>) {
        (
            name.into(),
            Ok(Poll {
                channel: Channel {
                    login: name.into(),
                    id: name.into(),
                    broadcast_id: online.then(|| "stream".into()),
                    game_id: None,
                    game_name: None,
                },
                points: points.then_some(Points {
                    balance: 0,
                    available_claim: None,
                }),
                drops: Ok(drops),
            }),
        )
    }

    fn drop_for(campaign: &str, group: &str) -> Drop {
        Drop {
            campaign_id: campaign.into(),
            group_id: group.into(),
            campaign: campaign.into(),
            reward: group.into(),
            minutes: Some(1),
            required: 60,
            grants: Some(0),
            repeats: None,
            status: DropStatus::Watching,
        }
    }

    fn catalog_for(results: &[(String, Result<Poll>)]) -> Vec<crate::drops::CatalogEntry> {
        results
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok())
            .flat_map(|p| p.drops.as_ref().unwrap())
            .map(|drop| crate::drops::CatalogEntry {
                drop: drop.clone(),
                channels: vec![],
                categories: vec![],
                ends_at: chrono::Utc::now(),
                link_url: None,
            })
            .collect()
    }
    fn names(selection: &Selection) -> Vec<&str> {
        selection
            .channels
            .iter()
            .map(|c| c.login.as_str())
            .collect()
    }

    #[test]
    fn priority_fills_free_slots_with_the_other_kind_of_farming() {
        let results = vec![
            poll("first", true, true, vec![]),
            poll("second", true, true, vec![]),
            poll("genshin", true, false, vec![drop_for("genshin", "one")]),
            poll("gta", true, false, vec![drop_for("gta", "one")]),
        ];
        let catalog = catalog_for(&results);
        for (priority, configured, max, expected, drop) in [
            (
                FarmingPriority::Points,
                vec!["first", "second"],
                2,
                vec!["first", "second"],
                None,
            ),
            (
                FarmingPriority::Points,
                vec!["first"],
                2,
                vec!["first", "genshin"],
                Some("genshin"),
            ),
            (
                FarmingPriority::Drops,
                vec!["first", "second"],
                2,
                vec!["genshin", "first"],
                Some("genshin"),
            ),
            (
                FarmingPriority::Points,
                vec![],
                2,
                vec!["genshin"],
                Some("genshin"),
            ),
            (
                FarmingPriority::Drops,
                vec![],
                2,
                vec!["genshin"],
                Some("genshin"),
            ),
            (
                FarmingPriority::Points,
                vec!["first"],
                1,
                vec!["first"],
                None,
            ),
            (
                FarmingPriority::Drops,
                vec!["first"],
                1,
                vec!["genshin"],
                Some("genshin"),
            ),
        ] {
            let configured: Vec<_> = configured.into_iter().map(str::to_string).collect();
            let selection = select_channels(
                &results,
                &configured,
                &catalog,
                &HashSet::new(),
                max,
                priority,
            );
            assert_eq!(names(&selection), expected);
            assert_eq!(selection.drop_channel.as_deref(), drop);
        }
    }

    #[test]
    fn one_channel_can_serve_points_and_drops_without_using_two_slots() {
        let results = vec![
            poll("both", true, true, vec![drop_for("one", "one")]),
            poll("points", true, true, vec![]),
            poll("other_drop", true, false, vec![drop_for("two", "one")]),
        ];
        for priority in [FarmingPriority::Points, FarmingPriority::Drops] {
            let selection = select_channels(
                &results,
                &["both".into(), "points".into()],
                &catalog_for(&results),
                &HashSet::new(),
                2,
                priority,
            );
            assert_eq!(names(&selection), ["both", "points"]);
            assert_eq!(selection.drop_channel.as_deref(), Some("both"));
        }
    }

    #[test]
    fn unavailable_campaigns_and_failed_streams_do_not_take_slots() {
        let results = vec![
            poll("failed", true, true, vec![]),
            poll("points", true, true, vec![]),
            poll("genshin", true, false, vec![drop_for("genshin", "one")]),
        ];
        let mut catalog = catalog_for(&results);
        catalog[0].drop.status = DropStatus::Claimed;
        let selection = select_channels(
            &results,
            &["failed".into(), "points".into()],
            &catalog,
            &HashSet::from(["failed".into()]),
            2,
            FarmingPriority::Drops,
        );
        assert_eq!(names(&selection), ["points"]);
        assert!(selection.drop_channel.is_none());
    }

    #[test]
    fn completed_daily_window_leaves_the_drops_slot_for_another_campaign() {
        let mut daily = drop_for("daily", "reward");
        daily.minutes = Some(60);
        daily.grants = Some(1);
        daily.repeats = Some(6);
        daily.status = DropStatus::Waiting(chrono::Utc::now() + chrono::Duration::hours(4));
        let results = vec![
            poll("daily", true, false, vec![daily]),
            poll("next", true, false, vec![drop_for("next", "reward")]),
        ];
        let catalog = catalog_for(&results);
        for priority in [FarmingPriority::Points, FarmingPriority::Drops] {
            let selection = select_channels(&results, &[], &catalog, &HashSet::new(), 2, priority);
            assert_eq!(names(&selection), ["next"]);
            assert_eq!(selection.drop_channel.as_deref(), Some("next"));
        }
    }

    #[test]
    fn no_drops_uses_normal_slots_and_offline_channels_are_excluded() {
        let results = vec![
            poll("offline", false, true, vec![]),
            poll("first", true, true, vec![]),
            poll("disabled", true, false, vec![]),
            poll("second", true, true, vec![]),
        ];
        let configured = vec![
            "offline".into(),
            "first".into(),
            "disabled".into(),
            "second".into(),
        ];
        for priority in [FarmingPriority::Points, FarmingPriority::Drops] {
            assert_eq!(
                names(&select_channels(
                    &results,
                    &configured,
                    &[],
                    &HashSet::new(),
                    2,
                    priority
                )),
                ["first", "second"]
            );
            assert_eq!(
                select_channels(&results, &configured, &[], &HashSet::new(), 1, priority)
                    .channels
                    .len(),
                1
            );
        }
    }
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
