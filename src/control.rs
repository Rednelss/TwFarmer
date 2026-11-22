use crate::{
    accounts::Profile,
    config::Config,
    drops::{CatalogEntry, DropStatus},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::HashSet, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Start,
    Stop,
    Status,
    Campaigns,
    Accounts,
    Login,
    Renew,
    Remove,
    Common,
    Own,
    Inherit,
    Temporary,
    Settings,
    Language,
    Config,
    Reload,
    Log,
    PlainLog,
    Help,
    Quit,
}

impl MenuItem {
    pub const ALL: [Self; 20] = [
        Self::Start,
        Self::Stop,
        Self::Status,
        Self::Campaigns,
        Self::Accounts,
        Self::Login,
        Self::Renew,
        Self::Remove,
        Self::Common,
        Self::Own,
        Self::Inherit,
        Self::Temporary,
        Self::Settings,
        Self::Language,
        Self::Config,
        Self::Reload,
        Self::Log,
        Self::PlainLog,
        Self::Help,
        Self::Quit,
    ];
    pub fn group(self) -> &'static str {
        match self {
            Self::Start | Self::Stop | Self::Status | Self::Campaigns => tr!("Farming", "Фарм"),
            Self::Accounts | Self::Login | Self::Renew | Self::Remove => {
                tr!("Accounts", "Аккаунты")
            }
            Self::Common | Self::Own | Self::Inherit | Self::Temporary => tr!("Channels", "Каналы"),
            Self::Settings | Self::Language | Self::Config | Self::Reload => {
                tr!("Settings", "Настройки")
            }
            _ => tr!("View", "Вид"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => tr!("Start selected accounts", "Запустить выбранные аккаунты"),
            Self::Stop => tr!("Stop farming", "Остановить фарм"),
            Self::Status => tr!("Check balances", "Проверить балансы"),
            Self::Campaigns => tr!("Available Drops campaigns", "Доступные кампании Drops"),
            Self::Accounts => tr!("Select accounts", "Выбрать аккаунты"),
            Self::Login => tr!("Add a Twitch account", "Добавить аккаунт Twitch"),
            Self::Renew => tr!("Renew account login", "Обновить вход в аккаунт"),
            Self::Remove => tr!("Remove an account", "Удалить аккаунт"),
            Self::Common => tr!("Shared channels", "Общие каналы"),
            Self::Own => tr!("Personal account channels", "Личные каналы аккаунта"),
            Self::Inherit => tr!(
                "Use shared channels for an account",
                "Вернуть аккаунт к общим каналам"
            ),
            Self::Temporary => tr!(
                "Channels for this run only",
                "Каналы только на текущий запуск"
            ),
            Self::Settings => tr!("Farming settings", "Настройки фарма"),
            Self::Language => "Language / Язык",
            Self::Config => tr!(
                "Open / create a configuration",
                "Открыть / создать конфигурацию"
            ),
            Self::Reload => tr!("Reload settings from disk", "Перечитать настройки с диска"),
            Self::Log => tr!("Show / hide the log", "Показать / скрыть журнал"),
            Self::PlainLog => tr!("Switch to plain log", "Переключиться на обычный журнал"),
            Self::Help => tr!("Help", "Помощь"),
            Self::Quit => tr!("Quit", "Выход"),
        }
    }
    #[cfg(test)]
    fn index(self) -> usize {
        Self::ALL.iter().position(|item| *item == self).unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingField {
    Channels,
    Poll,
    Watch,
    Bonus,
    Client,
    Scopes,
    Language,
    Chat,
    FarmDrops,
    ClaimDrops,
    AutoDrops,
    Priority,
}
impl SettingField {
    pub const ALL: [Self; 12] = [
        Self::Channels,
        Self::Poll,
        Self::Watch,
        Self::Bonus,
        Self::Client,
        Self::Scopes,
        Self::Language,
        Self::Chat,
        Self::FarmDrops,
        Self::ClaimDrops,
        Self::AutoDrops,
        Self::Priority,
    ];
    const DISPLAY_ORDER: [Self; 12] = [
        Self::Priority,
        Self::Channels,
        Self::FarmDrops,
        Self::AutoDrops,
        Self::ClaimDrops,
        Self::Bonus,
        Self::Chat,
        Self::Language,
        Self::Poll,
        Self::Watch,
        Self::Client,
        Self::Scopes,
    ];
    pub fn visible(advanced: bool) -> &'static [Self] {
        &Self::DISPLAY_ORDER[..if advanced { Self::ALL.len() } else { 8 }]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Channels => tr!(
                "Concurrent channels per account",
                "Каналов одновременно на аккаунт"
            ),
            Self::Poll => tr!(
                "Balance polling, seconds (30-3600)",
                "Опрос балансов, секунд (30-3600)"
            ),
            Self::Watch => tr!(
                "Watching interval, seconds (15-60)",
                "Просмотр, секунд (15-60)"
            ),
            Self::Bonus => tr!("Claim bonuses", "Забирать бонусы"),
            Self::Client => "Twitch Client ID",
            Self::Scopes => tr!(
                "OAuth scopes separated by spaces",
                "OAuth scopes через пробел"
            ),
            Self::Language => tr!("Language", "Язык"),
            Self::Chat => tr!("Join chat", "Присутствие в чате"),
            Self::FarmDrops => tr!("Farm Drops", "Фармить Drops"),
            Self::ClaimDrops => tr!("Claim Drops", "Забирать Drops"),
            Self::AutoDrops => tr!(
                "Find Drops channels automatically",
                "Автопоиск каналов Drops"
            ),
            Self::Priority => tr!("Farming priority", "Приоритет фарма"),
        }
    }
    pub fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Channels => &["1", "2"],
            Self::Language => &["en", "ru"],
            Self::Priority => &["points", "drops"],
            Self::Bonus | Self::Chat | Self::FarmDrops | Self::ClaimDrops | Self::AutoDrops => {
                &["false", "true"]
            }
            _ => &[],
        }
    }
    pub fn display(self, value: &str) -> String {
        match (self, value) {
            (Self::Language, "en") => "English".into(),
            (Self::Language, "ru") => "Русский".into(),
            (Self::Priority, "points") => tr!("Points first", "Сначала баллы").into(),
            (Self::Priority, "drops") => tr!("Drops first", "Сначала Drops").into(),
            (_, "true") if !self.choices().is_empty() => tr!("On", "Вкл").into(),
            (_, "false") if !self.choices().is_empty() => tr!("Off", "Выкл").into(),
            _ => value.into(),
        }
    }
    fn value(self, config: &Config) -> String {
        match self {
            Self::Channels => config.max_channels.to_string(),
            Self::Poll => config.poll_interval_seconds.to_string(),
            Self::Watch => config.watch_interval_seconds.to_string(),
            Self::Bonus => config.claim_bonus.to_string(),
            Self::Client => config.client_id.clone(),
            Self::Scopes => config.oauth_scopes.join(" "),
            Self::Language => config.language.code().into(),
            Self::Chat => config.chat_presence.to_string(),
            Self::FarmDrops => config.farm_drops.to_string(),
            Self::ClaimDrops => config.claim_drops.to_string(),
            Self::AutoDrops => config.auto_drops.to_string(),
            Self::Priority => config.farming_priority.code().into(),
        }
    }
}

pub fn visible_campaigns(
    entries: &[(String, CatalogEntry)],
    show_all: bool,
) -> impl Iterator<Item = &(String, CatalogEntry)> {
    entries.iter().filter(move |(_, entry)| {
        show_all || entry.drop.pending() || entry.drop.status == DropStatus::Claimable
    })
}

#[derive(Clone)]
pub enum EditTarget {
    Common,
    Own(String),
    Temporary,
    Config,
}

#[derive(Clone)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self { text, cursor }
    }
    fn byte(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(at, _)| at)
    }
    pub fn insert(&mut self, text: &str) {
        let text = text.replace(['\r', '\n'], " ");
        self.text.insert_str(self.byte(self.cursor), &text);
        self.cursor += text.chars().count();
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            KeyCode::Backspace if self.cursor > 0 => {
                let end = self.byte(self.cursor);
                self.cursor -= 1;
                let start = self.byte(self.cursor);
                self.text.replace_range(start..end, "");
            }
            KeyCode::Delete if self.cursor < self.text.chars().count() => {
                self.text
                    .replace_range(self.byte(self.cursor)..self.byte(self.cursor + 1), "");
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
pub enum Pick {
    Renew,
    Remove,
    Own,
    Inherit,
}

pub enum Overlay {
    Campaigns {
        index: usize,
        entries: Vec<(String, CatalogEntry)>,
        errors: Vec<String>,
        show_all: bool,
        expanded: bool,
        scroll: u16,
    },
    CampaignsLoading,
    Language(usize),
    Menu(usize),
    Accounts {
        index: usize,
        chosen: HashSet<String>,
    },
    Pick {
        kind: Pick,
        index: usize,
    },
    Confirm {
        name: String,
        yes: bool,
    },
    Edit {
        target: EditTarget,
        editor: Editor,
    },
    Settings {
        index: usize,
        editors: Vec<Editor>,
        advanced: bool,
    },
    Login {
        uri: String,
        code: String,
    },
    Loading,
    Help(u16),
}

pub enum Action {
    Campaigns,
    SetLanguage(crate::i18n::Language),
    Start,
    Stop,
    Status,
    Login(Option<String>),
    Remove(String),
    SaveEdit(EditTarget, String),
    SaveSelection(HashSet<String>),
    UseSelection(HashSet<String>),
    Inherit(String),
    SaveSettings(Vec<String>),
    ToggleLog,
    Quit,
    CancelJob,
    PlainLog,
    Reload,
}

#[derive(Default)]
pub struct Controls {
    pub overlay: Option<Overlay>,
    pub notice: String,
    pub log_only: bool,
    pub plain_log: bool,
}

impl Controls {
    pub fn menu(&mut self) {
        self.overlay = Some(Overlay::Menu(0));
    }
    pub fn edit(&mut self, target: EditTarget, text: String) {
        self.overlay = Some(Overlay::Edit {
            target,
            editor: Editor::new(text),
        });
    }
    pub fn settings(&mut self, config: &Config) {
        self.overlay = Some(Overlay::Settings {
            index: SettingField::Priority as usize,
            editors: SettingField::ALL
                .into_iter()
                .map(|field| Editor::new(field.value(config)))
                .collect(),
            advanced: false,
        });
    }
    pub fn paste(&mut self, text: &str) {
        match &mut self.overlay {
            Some(Overlay::Edit { editor, .. }) => editor.insert(text),
            Some(Overlay::Settings { index, editors, .. })
                if SettingField::ALL[*index].choices().is_empty() =>
            {
                editors[*index].insert(text)
            }
            _ => {}
        }
    }
    pub fn key(
        &mut self,
        key: KeyEvent,
        profiles: &[Profile],
        config: &Config,
        selected: &HashSet<String>,
    ) -> Option<Action> {
        if key.code == KeyCode::Char('q')
            && matches!(
                self.overlay,
                None | Some(Overlay::Menu(_)) | Some(Overlay::Help(_))
            )
        {
            return Some(Action::Quit);
        }
        if key.code == KeyCode::Esc {
            let cancel = matches!(
                self.overlay,
                Some(Overlay::Login { .. })
                    | Some(Overlay::Loading)
                    | Some(Overlay::CampaignsLoading)
            );
            self.overlay = None;
            return cancel.then_some(Action::CancelJob);
        }
        match &mut self.overlay {
            Some(Overlay::Menu(index)) => match key.code {
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down | KeyCode::Tab => *index = (*index + 1) % MenuItem::ALL.len(),
                KeyCode::BackTab => {
                    *index = (*index + MenuItem::ALL.len() - 1) % MenuItem::ALL.len()
                }
                KeyCode::Enter => {
                    let item = MenuItem::ALL[*index];
                    self.overlay = None;
                    match item {
                        MenuItem::Start => return Some(Action::Start),
                        MenuItem::Stop => return Some(Action::Stop),
                        MenuItem::Status => return Some(Action::Status),
                        MenuItem::Accounts => {
                            self.overlay = Some(Overlay::Accounts {
                                index: 0,
                                chosen: selected.clone(),
                            })
                        }
                        MenuItem::Login => return Some(Action::Login(None)),
                        MenuItem::Renew | MenuItem::Remove | MenuItem::Own | MenuItem::Inherit => {
                            if profiles.is_empty() {
                                self.notice = tr!(
                                    "Add a Twitch account first.",
                                    "Сначала добавьте аккаунт Twitch."
                                )
                                .into();
                            } else {
                                self.overlay = Some(Overlay::Pick {
                                    index: 0,
                                    kind: match item {
                                        MenuItem::Renew => Pick::Renew,
                                        MenuItem::Remove => Pick::Remove,
                                        MenuItem::Own => Pick::Own,
                                        _ => Pick::Inherit,
                                    },
                                });
                            }
                        }
                        MenuItem::Common => {
                            self.edit(EditTarget::Common, config.channels.join(" "))
                        }
                        MenuItem::Temporary => self.edit(EditTarget::Temporary, String::new()),
                        MenuItem::Settings => self.settings(config),
                        MenuItem::Config => self.edit(EditTarget::Config, String::new()),
                        MenuItem::Log => return Some(Action::ToggleLog),
                        MenuItem::Help => self.overlay = Some(Overlay::Help(0)),
                        MenuItem::Quit => return Some(Action::Quit),
                        MenuItem::PlainLog => return Some(Action::PlainLog),
                        MenuItem::Reload => return Some(Action::Reload),
                        MenuItem::Campaigns => return Some(Action::Campaigns),
                        MenuItem::Language => {
                            self.overlay = Some(Overlay::Language(usize::from(
                                config.language == crate::i18n::Language::Ru,
                            )));
                        }
                    }
                }
                _ => {}
            },
            Some(Overlay::Accounts { index, chosen }) => match key.code {
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => *index = (*index + 1).min(profiles.len().saturating_sub(1)),
                KeyCode::Char(' ') => {
                    if let Some(profile) = profiles.get(*index)
                        && !chosen.remove(&profile.name)
                    {
                        chosen.insert(profile.name.clone());
                    }
                }
                KeyCode::Char('a') => {
                    *chosen = profiles.iter().map(|p| p.name.clone()).collect();
                }
                KeyCode::Char('n') => chosen.clear(),
                KeyCode::Enter => return Some(Action::SaveSelection(chosen.clone())),
                KeyCode::Char('t') => return Some(Action::UseSelection(chosen.clone())),
                _ => {}
            },
            Some(Overlay::Pick { kind, index }) => match key.code {
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => *index = (*index + 1).min(profiles.len().saturating_sub(1)),
                KeyCode::Enter => {
                    if let Some(profile) = profiles.get(*index) {
                        let name = profile.name.clone();
                        match kind {
                            Pick::Renew => return Some(Action::Login(Some(name))),
                            Pick::Remove => {
                                self.overlay = Some(Overlay::Confirm { name, yes: false })
                            }
                            Pick::Own => self.edit(
                                EditTarget::Own(name),
                                config.channels_for(&profile.config_key).join(" "),
                            ),
                            Pick::Inherit => return Some(Action::Inherit(name)),
                        }
                    }
                }
                _ => {}
            },
            Some(Overlay::Confirm { name, yes }) => match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => *yes = !*yes,
                KeyCode::Enter if *yes => return Some(Action::Remove(name.clone())),
                KeyCode::Enter => self.overlay = None,
                _ => {}
            },
            Some(Overlay::Edit { target, editor }) => {
                if key.code == KeyCode::Enter {
                    return Some(Action::SaveEdit(target.clone(), editor.text.clone()));
                }
                editor.key(key);
            }
            Some(Overlay::Settings {
                index,
                editors,
                advanced,
            }) => {
                let fields = SettingField::visible(*advanced);
                let position = fields
                    .iter()
                    .position(|field| *field as usize == *index)
                    .unwrap_or(0);
                let choices = SettingField::ALL[*index].choices();
                match key.code {
                    KeyCode::F(2) => {
                        *advanced = !*advanced;
                        if !SettingField::visible(*advanced).contains(&SettingField::ALL[*index]) {
                            *index = SettingField::Priority as usize;
                        }
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        *index = fields[(position + 1) % fields.len()] as usize
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        *index = fields[(position + fields.len() - 1) % fields.len()] as usize
                    }
                    KeyCode::Enter => {
                        return Some(Action::SaveSettings(
                            editors.iter().map(|e| e.text.clone()).collect(),
                        ));
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if !choices.is_empty() => {
                        let current = choices
                            .iter()
                            .position(|value| *value == editors[*index].text)
                            .unwrap_or(0);
                        let offset = if key.code == KeyCode::Left {
                            choices.len() - 1
                        } else {
                            1
                        };
                        editors[*index] =
                            Editor::new(choices[(current + offset) % choices.len()].into());
                    }
                    _ if choices.is_empty() => editors[*index].key(key),
                    _ => {}
                }
            }
            Some(Overlay::Language(index)) => match key.code {
                KeyCode::Up | KeyCode::Left => *index = 0,
                KeyCode::Down | KeyCode::Right => *index = 1,
                KeyCode::Enter => {
                    return Some(Action::SetLanguage(if *index == 1 {
                        crate::i18n::Language::Ru
                    } else {
                        crate::i18n::Language::En
                    }));
                }
                _ => {}
            },
            Some(Overlay::Campaigns {
                index,
                entries,
                show_all,
                expanded,
                scroll,
                ..
            }) => match key.code {
                KeyCode::Char('r') => return Some(Action::Campaigns),
                KeyCode::Tab => {
                    *show_all = !*show_all;
                    *index = 0;
                    *scroll = 0;
                }
                KeyCode::Enter => {
                    *expanded = !*expanded;
                    *scroll = 0;
                }
                KeyCode::Up if *expanded => *scroll = scroll.saturating_sub(1),
                KeyCode::Down if *expanded => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp if *expanded => *scroll = scroll.saturating_sub(10),
                KeyCode::PageDown if *expanded => *scroll = scroll.saturating_add(10),
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => {
                    *index = (*index + 1).min(
                        visible_campaigns(entries, *show_all)
                            .count()
                            .saturating_sub(1),
                    )
                }
                KeyCode::PageDown => {
                    *index = (*index + 10).min(
                        visible_campaigns(entries, *show_all)
                            .count()
                            .saturating_sub(1),
                    )
                }
                KeyCode::PageUp => *index = index.saturating_sub(10),
                _ => {}
            },
            Some(Overlay::Help(scroll)) => match key.code {
                KeyCode::Up => *scroll = scroll.saturating_sub(1),
                KeyCode::Down => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                KeyCode::Enter => self.overlay = None,
                _ => {}
            },
            Some(Overlay::Login { .. })
            | Some(Overlay::Loading)
            | Some(Overlay::CampaignsLoading) => {}
            None => match key.code {
                KeyCode::Char('m') | KeyCode::F(1) => self.menu(),
                KeyCode::Char('s') => return Some(Action::Start),
                KeyCode::Char('x') => return Some(Action::Stop),
                KeyCode::Char('a') => return Some(Action::Login(None)),
                KeyCode::Char('c') => self.edit(EditTarget::Common, config.channels.join(" ")),
                KeyCode::Char('l') => return Some(Action::ToggleLog),
                KeyCode::Char('d') => return Some(Action::Campaigns),
                KeyCode::Char('p') => self.settings(config),
                KeyCode::Char('?') => self.overlay = Some(Overlay::Help(0)),
                KeyCode::Char('q') => return Some(Action::Quit),
                _ => {}
            },
        }
        None
    }
}

pub fn settings(config: &Config, values: &[String]) -> anyhow::Result<Config> {
    use anyhow::{Context, ensure};
    ensure!(
        matches!(values.len(), 6..=12),
        "{}",
        tf!("Missing settings fields", "Не хватает полей настроек")
    );
    crate::config::SettingsPatch {
        farming_priority: values
            .get(SettingField::Priority as usize)
            .map(|v| match v.trim() {
                "points" => Ok(crate::config::FarmingPriority::Points),
                "drops" => Ok(crate::config::FarmingPriority::Drops),
                _ => Err(anyhow::anyhow!(
                    "{}",
                    tr!(
                        "Use points or drops for priority",
                        "Для приоритета укажите points или drops"
                    )
                )),
            })
            .transpose()?,
        auto_drops: values
            .get(SettingField::AutoDrops as usize)
            .map(|v| v.trim().parse())
            .transpose()
            .context(tr!(
                "Use true or false for automatic Drops",
                "Для автопоиска Drops укажите true или false"
            ))?,
        farm_drops: values
            .get(SettingField::FarmDrops as usize)
            .map(|v| v.trim().parse())
            .transpose()
            .context(tr!(
                "Use true or false for farming Drops",
                "Для фарма Drops укажите true или false"
            ))?,
        claim_drops: values
            .get(SettingField::ClaimDrops as usize)
            .map(|v| v.trim().parse())
            .transpose()
            .context(tr!(
                "Use true or false for claiming Drops",
                "Для сбора Drops укажите true или false"
            ))?,
        chat_presence: values
            .get(SettingField::Chat as usize)
            .map(|value| value.trim().parse())
            .transpose()
            .context(tr!(
                "Use true or false for chat presence",
                "Для присутствия в чате укажите true или false"
            ))?,
        language: values
            .get(SettingField::Language as usize)
            .map(|value| value.parse())
            .transpose()
            .map_err(anyhow::Error::msg)?,
        max_channels: Some(
            values[SettingField::Channels as usize]
                .trim()
                .parse()
                .context(tr!(
                    "Enter the channel count as a number",
                    "Укажите количество каналов числом"
                ))?,
        ),
        poll_interval_seconds: Some(values[SettingField::Poll as usize].trim().parse().context(
            tr!(
                "Enter the polling interval as a number",
                "Укажите интервал опроса числом"
            ),
        )?),
        watch_interval_seconds: Some(
            values[SettingField::Watch as usize]
                .trim()
                .parse()
                .context(tr!(
                    "Enter the watching interval as a number",
                    "Укажите интервал просмотра числом"
                ))?,
        ),
        claim_bonus: Some(
            values[SettingField::Bonus as usize]
                .trim()
                .parse()
                .context(tr!(
                    "Use true or false for claiming bonuses",
                    "Для бонусов укажите true или false"
                ))?,
        ),
        client_id: Some(values[SettingField::Client as usize].clone()),
        oauth_scopes: Some(values[SettingField::Scopes as usize].clone()),
    }
    .apply(config)
}

pub fn config_path(text: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !text.trim().is_empty(),
        "{}",
        tf!(
            "Specify the configuration path",
            "Укажите путь к конфигурации"
        )
    );
    Ok(PathBuf::from(text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(controls: &mut Controls, code: KeyCode, config: &Config) -> Option<Action> {
        controls.key(
            KeyEvent::new(code, KeyModifiers::NONE),
            &[],
            config,
            &HashSet::new(),
        )
    }
    fn focus(controls: &mut Controls, field: SettingField, config: &Config) {
        for _ in 0..SettingField::ALL.len() {
            if matches!(controls.overlay, Some(Overlay::Settings { index, .. }) if index == field as usize)
            {
                return;
            }
            press(controls, KeyCode::Tab, config);
        }
        panic!("Field {field:?} is unreachable");
    }
    fn campaign(status: DropStatus) -> (String, CatalogEntry) {
        (
            "account".into(),
            CatalogEntry {
                drop: crate::drops::Drop {
                    campaign_id: "campaign".into(),
                    group_id: "group".into(),
                    campaign: "Campaign".into(),
                    reward: "Reward".into(),
                    minutes: Some(5),
                    required: 30,
                    grants: None,
                    repeats: None,
                    status,
                },
                channels: vec!["channel".into()],
                categories: vec![],
                ends_at: chrono::Utc::now(),
                link_url: None,
            },
        )
    }
    #[test]
    fn priority_can_be_changed_from_the_settings_form() {
        let config = Config::default();
        let selected = HashSet::new();
        let mut controls = Controls {
            overlay: Some(Overlay::Menu(MenuItem::Settings.index())),
            ..Controls::default()
        };
        controls.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        );
        press(&mut controls, KeyCode::Right, &config);
        let Some(Action::SaveSettings(mut values)) = controls.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        ) else {
            panic!("Settings did not save");
        };
        assert_eq!(
            settings(&config, &values).unwrap().farming_priority,
            crate::config::FarmingPriority::Drops
        );
        values[11] = "invalid".into();
        assert!(settings(&config, &values).is_err());
        assert_eq!(
            config.farming_priority,
            crate::config::FarmingPriority::Points
        );
    }
    #[test]
    fn campaigns_menu_and_refresh_are_available_from_the_keyboard() {
        let mut controls = Controls {
            overlay: Some(Overlay::Menu(MenuItem::Campaigns.index())),
            ..Controls::default()
        };
        let config = Config::default();
        let selected = HashSet::new();
        assert!(matches!(
            controls.key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &[],
                &config,
                &selected
            ),
            Some(Action::Campaigns)
        ));
        controls.overlay = Some(Overlay::Campaigns {
            index: 0,
            entries: vec![
                campaign(DropStatus::Watching),
                campaign(DropStatus::LinkRequired),
            ],
            errors: vec![],
            show_all: false,
            expanded: false,
            scroll: 0,
        });
        assert_eq!(
            visible_campaigns(&[campaign(DropStatus::Claimable)], false).count(),
            1
        );
        press(&mut controls, KeyCode::PageDown, &config);
        assert!(matches!(
            controls.overlay,
            Some(Overlay::Campaigns { index: 0, .. })
        ));
        press(&mut controls, KeyCode::Tab, &config);
        controls.key(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        );
        assert!(matches!(
            controls.overlay,
            Some(Overlay::Campaigns { index: 1, .. })
        ));
        press(&mut controls, KeyCode::Enter, &config);
        press(&mut controls, KeyCode::PageDown, &config);
        assert!(matches!(
            controls.overlay,
            Some(Overlay::Campaigns {
                index: 1,
                expanded: true,
                scroll: 10,
                ..
            })
        ));
        press(&mut controls, KeyCode::Enter, &config);
        assert!(matches!(
            controls.overlay,
            Some(Overlay::Campaigns {
                index: 1,
                expanded: false,
                scroll: 0,
                ..
            })
        ));
        assert!(matches!(
            controls.key(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                &[],
                &config,
                &selected
            ),
            Some(Action::Campaigns)
        ));
        controls.key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        );
        assert!(controls.overlay.is_none());
    }
    #[test]
    fn drops_switches_can_be_reached_and_saved_with_the_keyboard() {
        let config = Config::default();
        let mut controls = Controls {
            overlay: Some(Overlay::Menu(MenuItem::Settings.index())),
            ..Controls::default()
        };
        let selected = HashSet::new();
        controls.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        );
        focus(&mut controls, SettingField::FarmDrops, &config);
        press(&mut controls, KeyCode::Char(' '), &config);
        focus(&mut controls, SettingField::ClaimDrops, &config);
        press(&mut controls, KeyCode::Left, &config);
        let action = controls.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &[],
            &config,
            &selected,
        );
        let Some(Action::SaveSettings(values)) = action else {
            panic!("Settings form did not save");
        };
        let next = settings(&config, &values).unwrap();
        assert!(!next.farm_drops);
        assert!(!next.claim_drops);
        assert!(next.chat_presence);
    }
    #[test]
    fn advanced_fields_are_reachable_and_kept_when_hidden() {
        let config = Config::default();
        let mut controls = Controls::default();
        controls.settings(&config);
        press(&mut controls, KeyCode::F(2), &config);
        for field in SettingField::ALL {
            focus(&mut controls, field, &config);
        }
        focus(&mut controls, SettingField::Poll, &config);
        controls.key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &[],
            &config,
            &HashSet::new(),
        );
        controls.paste("120");
        press(&mut controls, KeyCode::F(2), &config);
        assert!(
            matches!(controls.overlay, Some(Overlay::Settings { index, advanced: false, .. }) if index == SettingField::Priority as usize)
        );
        let Some(Action::SaveSettings(values)) = press(&mut controls, KeyCode::Enter, &config)
        else {
            panic!("Settings did not save");
        };
        let next = settings(&config, &values).unwrap();
        assert_eq!(next.poll_interval_seconds, 120);
        assert_eq!(next.client_id, config.client_id);
        assert_eq!(next.oauth_scopes, config.oauth_scopes);
    }
    #[test]
    fn drops_settings_are_editable_without_changing_old_form_values() {
        let config = Config::default();
        let values = [
            "2",
            "60",
            "20",
            "true",
            crate::config::CLIENT_ID,
            "user_read",
            "en",
            "true",
            "false",
            "false",
        ]
        .map(str::to_string);
        let next = settings(&config, &values).unwrap();
        assert!(!next.farm_drops);
        assert!(!next.claim_drops);
        assert!(settings(&config, &values[..8]).unwrap().farm_drops);
        let mut automatic = values.to_vec();
        automatic.push("true".into());
        automatic[8] = "true".into();
        assert!(settings(&config, &automatic).unwrap().automatic_drops());
        let mut invalid = values;
        invalid[9] = "maybe".into();
        assert!(settings(&config, &invalid).is_err());
    }
    #[test]
    fn chat_presence_is_editable_and_invalid_boolean_is_rejected() {
        let config = Config::default();
        let values = [
            "2",
            "60",
            "20",
            "true",
            crate::config::CLIENT_ID,
            "user_read",
            "en",
            "false",
        ]
        .map(str::to_string);
        assert!(!settings(&config, &values).unwrap().chat_presence);
        assert!(config.chat_presence);
        let mut invalid = values;
        invalid[7] = "maybe".into();
        assert!(settings(&config, &invalid).is_err());
    }
    use crossterm::event::KeyModifiers;
    #[test]
    fn editing_handles_unicode_cursor_and_pasted_newlines() {
        let mut editor = Editor::new("аб".into());
        editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        editor.insert("X\nY");
        assert_eq!(editor.text, "аX Yб");
        editor.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(editor.text, "аX б");
        editor.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(editor.text, "аX ");
    }
    #[test]
    fn account_selection_can_be_empty_and_delete_requires_confirmation() {
        let profile = Profile {
            name: "twitch_user".into(),
            login: Some("twitch_user".into()),
            path: PathBuf::from("/tmp/session"),
            config_key: "twitch_user".into(),
        };
        let profiles = vec![profile];
        let mut controls = Controls {
            overlay: Some(Overlay::Accounts {
                index: 0,
                chosen: HashSet::new(),
            }),
            ..Controls::default()
        };
        let action = controls.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &profiles,
            &Config::default(),
            &HashSet::new(),
        );
        assert!(matches!(action,Some(Action::SaveSelection(chosen)) if chosen.is_empty()));
        controls.overlay = Some(Overlay::Confirm {
            name: "twitch_user".into(),
            yes: false,
        });
        assert!(
            controls
                .key(
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    &profiles,
                    &Config::default(),
                    &HashSet::new()
                )
                .is_none()
        );
        assert!(controls.overlay.is_none());
    }
    #[test]
    fn settings_reject_values_outside_twitch_limits() {
        let config = Config::default();
        let values = vec![
            "3".into(),
            "60".into(),
            "20".into(),
            "true".into(),
            config.client_id.clone(),
            "user_read".into(),
        ];
        assert!(settings(&config, &values).is_err());
    }
}
