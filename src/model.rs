use std::{collections::VecDeque, time::Duration};
use tokio::{sync::mpsc, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    Idle,
    Connecting,
    Running,
    Waiting,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelStatus {
    Online,
    Idle,
    Checking,
    Offline,
    Queued,
    Starting,
    Watching,
    Disabled,
    Retry(Instant),
    Error,
}

impl ChannelStatus {
    pub fn sort_key(&self) -> u8 {
        match self {
            Self::Watching => 0,
            Self::Starting => 1,
            Self::Retry(_) => 2,
            Self::Online => 3,
            Self::Queued => 4,
            Self::Checking => 5,
            Self::Idle => 6,
            Self::Error => 7,
            Self::Disabled => 8,
            Self::Offline => 9,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Online => tr!("Online", "Онлайн").into(),
            Self::Idle => tr!("Stopped", "Остановлен").into(),
            Self::Checking => tr!("Checking", "Проверка").into(),
            Self::Offline => tr!("Offline", "Офлайн").into(),
            Self::Queued => tr!("Queued", "В очереди").into(),
            Self::Starting => tr!("Starting", "Запуск").into(),
            Self::Watching => tr!("Farming", "Фарм").into(),
            Self::Disabled => tr!("Points disabled", "Баллы отключены").into(),
            Self::Retry(at) => tf!(
                "Retry in {}s",
                "Повтор {}с",
                at.saturating_duration_since(Instant::now()).as_secs()
            ),
            Self::Error => tr!("Error", "Ошибка").into(),
        }
    }
}

#[derive(Clone)]
pub struct ChannelView {
    pub name: String,
    pub status: ChannelStatus,
    pub balance: Option<i64>,
    pub baseline: Option<i64>,
    pub bonuses: u64,
    pub active_time: Duration,
    pub last_update: Option<Instant>,
    pub last_success: Option<Instant>,
    pub error: Option<String>,
}

impl ChannelView {
    pub fn new(name: String) -> Self {
        Self {
            name,
            status: ChannelStatus::Checking,
            balance: None,
            baseline: None,
            bonuses: 0,
            active_time: Duration::ZERO,
            last_update: None,
            last_success: None,
            error: None,
        }
    }
    pub fn gain(&self) -> i64 {
        self.balance
            .zip(self.baseline)
            .map_or(0, |(now, initial)| now - initial)
    }
    pub fn set_balance(&mut self, balance: i64) {
        self.baseline.get_or_insert(balance);
        self.balance = Some(balance);
        self.last_update = Some(Instant::now());
    }
}

pub struct AccountView {
    pub name: String,
    pub login: Option<String>,
    pub status: AccountStatus,
    pub error: Option<String>,
    pub channels: Vec<ChannelView>,
}

pub struct LogEntry {
    pub at: Instant,
    pub profile: String,
    pub message: String,
    pub error: bool,
}

pub enum Event {
    Account {
        profile: String,
        login: Option<String>,
        status: AccountStatus,
        error: Option<String>,
    },
    Channel {
        profile: String,
        view: ChannelView,
    },
    Log {
        profile: String,
        message: String,
        error: bool,
    },
}

#[derive(Clone)]
pub struct Reporter {
    pub profile: String,
    sender: mpsc::UnboundedSender<Event>,
}

impl Reporter {
    pub fn new(profile: String, sender: mpsc::UnboundedSender<Event>) -> Self {
        Self { profile, sender }
    }
    pub fn account(&self, login: Option<String>, status: AccountStatus, error: Option<String>) {
        let _ = self.sender.send(Event::Account {
            profile: self.profile.clone(),
            login,
            status,
            error,
        });
    }
    pub fn channel(&self, view: &ChannelView) {
        let _ = self.sender.send(Event::Channel {
            profile: self.profile.clone(),
            view: view.clone(),
        });
    }
    pub fn log(&self, message: impl Into<String>, error: bool) {
        let _ = self.sender.send(Event::Log {
            profile: self.profile.clone(),
            message: message.into(),
            error,
        });
    }
}

pub struct Dashboard {
    pub accounts: Vec<AccountView>,
    pub logs: VecDeque<LogEntry>,
    pub started: Instant,
}

impl Dashboard {
    pub fn new(profiles: Vec<(String, Option<String>, Vec<String>)>) -> Self {
        Self {
            accounts: profiles
                .into_iter()
                .map(|(name, login, channels)| AccountView {
                    name,
                    login,
                    status: AccountStatus::Connecting,
                    error: None,
                    channels: channels.into_iter().map(ChannelView::new).collect(),
                })
                .collect(),
            logs: VecDeque::new(),
            started: Instant::now(),
        }
    }
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Account {
                profile,
                login,
                status,
                error,
            } => {
                if let Some(account) = self.accounts.iter_mut().find(|a| a.name == profile) {
                    if login.is_some() {
                        account.login = login;
                    }
                    account.status = status;
                    account.error = error;
                    if status == AccountStatus::Error {
                        for channel in &mut account.channels {
                            channel.status = ChannelStatus::Error;
                            channel.error = account.error.clone();
                        }
                    }
                }
            }
            Event::Channel { profile, view } => {
                if let Some(account) = self.accounts.iter_mut().find(|a| a.name == profile)
                    && let Some(channel) = account.channels.iter_mut().find(|c| c.name == view.name)
                {
                    *channel = view;
                }
            }
            Event::Log {
                profile,
                message,
                error,
            } => {
                self.logs.push_back(LogEntry {
                    at: Instant::now(),
                    profile,
                    message,
                    error,
                });
                while self.logs.len() > 200 {
                    self.logs.pop_front();
                }
            }
        }
    }
    pub fn rows(&self, filter: Option<usize>) -> Vec<(&AccountView, &ChannelView)> {
        let mut rows: Vec<_> = self
            .accounts
            .iter()
            .enumerate()
            .filter(|(index, _)| filter.is_none_or(|filter| filter == *index))
            .flat_map(|(_, account)| {
                account
                    .channels
                    .iter()
                    .map(move |channel| (account, channel))
            })
            .collect();
        rows.sort_by_key(|(_, channel)| channel.status.sort_key());
        rows
    }
    pub fn gain(&self) -> i64 {
        self.accounts
            .iter()
            .flat_map(|a| &a.channels)
            .map(ChannelView::gain)
            .sum()
    }
    pub fn bonuses(&self) -> u64 {
        self.accounts
            .iter()
            .flat_map(|a| &a.channels)
            .map(|c| c.bonuses)
            .sum()
    }
}

pub fn duration_text(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_group_states_globally_and_preserve_priority_within_each_state() {
        let mut dashboard = Dashboard::new(vec![
            (
                "one".into(),
                None,
                vec!["offline".into(), "first".into(), "second".into()],
            ),
            (
                "two".into(),
                None,
                vec!["retry".into(), "watching".into(), "online".into()],
            ),
        ]);
        dashboard.accounts[0].channels[0].status = ChannelStatus::Offline;
        dashboard.accounts[0].channels[1].status = ChannelStatus::Watching;
        dashboard.accounts[0].channels[2].status = ChannelStatus::Watching;
        dashboard.accounts[1].channels[0].status = ChannelStatus::Retry(Instant::now());
        dashboard.accounts[1].channels[1].status = ChannelStatus::Watching;
        dashboard.accounts[1].channels[2].status = ChannelStatus::Online;
        let names = |filter| {
            dashboard
                .rows(filter)
                .into_iter()
                .map(|(_, c)| c.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(None),
            ["first", "second", "watching", "retry", "online", "offline"]
        );
        assert_eq!(names(Some(0)), ["first", "second", "offline"]);
        assert_eq!(dashboard.accounts[0].channels[0].name, "offline");
    }

    #[test]
    fn the_same_channel_keeps_separate_balances_for_each_account() {
        let mut dashboard = Dashboard::new(vec![
            ("main".into(), None, vec!["channel".into()]),
            ("alt".into(), None, vec!["channel".into()]),
        ]);
        let mut first = ChannelView::new("channel".into());
        first.set_balance(100);
        first.set_balance(150);
        dashboard.apply(Event::Channel {
            profile: "main".into(),
            view: first,
        });
        let mut second = ChannelView::new("channel".into());
        second.set_balance(10);
        second.status = ChannelStatus::Watching;
        dashboard.apply(Event::Channel {
            profile: "alt".into(),
            view: second,
        });
        dashboard.apply(Event::Account {
            profile: "main".into(),
            login: None,
            status: AccountStatus::Error,
            error: Some("Session expired".into()),
        });
        assert_eq!(dashboard.gain(), 50);
        assert_eq!(dashboard.accounts[1].channels[0].balance, Some(10));
        assert_eq!(
            dashboard.accounts[1].channels[0].status,
            ChannelStatus::Watching
        );
        assert_eq!(dashboard.rows(Some(1)).len(), 1);
    }
    #[test]
    fn session_gain_tracks_spending_and_logs_are_bounded() {
        let mut view = ChannelView::new("channel".into());
        view.set_balance(100);
        view.set_balance(75);
        assert_eq!(view.gain(), -25);
        let mut dashboard = Dashboard::new(vec![]);
        for n in 0..250 {
            dashboard.apply(Event::Log {
                profile: "main".into(),
                message: n.to_string(),
                error: false,
            });
        }
        assert_eq!(dashboard.logs.len(), 200);
        assert_eq!(dashboard.logs.front().unwrap().message, "50");
    }
}
