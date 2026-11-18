use crate::{accounts::Profile, config::Config};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::HashSet, path::PathBuf};

pub const MENU: &[&str] = &[
    "Запустить фарм выбранных аккаунтов",
    "Остановить фарм",
    "Проверить балансы без фарма",
    "Выбрать аккаунты для запуска",
    "Добавить аккаунт Twitch",
    "Обновить вход в аккаунт",
    "Удалить аккаунт",
    "Общие каналы",
    "Личные каналы аккаунта",
    "Вернуть аккаунт к общим каналам",
    "Каналы только на текущий запуск",
    "Настройки фарма",
    "Открыть / создать конфигурацию",
    "Показать / скрыть журнал",
    "Помощь",
    "Выход",
    "Переключиться на обычный журнал",
    "Перечитать настройки с диска",
];
pub const FIELDS: &[&str] = &[
    "Каналов одновременно на аккаунт (1–2)",
    "Опрос балансов, секунд (30–3600)",
    "Просмотр, секунд (15–60)",
    "Забирать бонусы (true/false)",
    "Client ID Twitch",
    "OAuth scopes через пробел",
];

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
    },
    Login {
        uri: String,
        code: String,
    },
    Loading,
    Help,
}

pub enum Action {
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
    pub fn paste(&mut self, text: &str) {
        match &mut self.overlay {
            Some(Overlay::Edit { editor, .. }) => editor.insert(text),
            Some(Overlay::Settings { index, editors }) => editors[*index].insert(text),
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
                None | Some(Overlay::Menu(_)) | Some(Overlay::Help)
            )
        {
            return Some(Action::Quit);
        }
        if key.code == KeyCode::Esc {
            let cancel = matches!(
                self.overlay,
                Some(Overlay::Login { .. }) | Some(Overlay::Loading)
            );
            self.overlay = None;
            return cancel.then_some(Action::CancelJob);
        }
        match &mut self.overlay {
            Some(Overlay::Menu(index)) => match key.code {
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => *index = (*index + 1).min(MENU.len() - 1),
                KeyCode::Enter => {
                    let index = *index;
                    self.overlay = None;
                    match index {
                        0 => return Some(Action::Start),
                        1 => return Some(Action::Stop),
                        2 => return Some(Action::Status),
                        3 => {
                            self.overlay = Some(Overlay::Accounts {
                                index: 0,
                                chosen: selected.clone(),
                            })
                        }
                        4 => return Some(Action::Login(None)),
                        5..=6 | 8..=9 => {
                            if profiles.is_empty() {
                                self.notice = "Сначала добавьте аккаунт Twitch.".into();
                            } else {
                                self.overlay = Some(Overlay::Pick {
                                    index: 0,
                                    kind: match index {
                                        5 => Pick::Renew,
                                        6 => Pick::Remove,
                                        8 => Pick::Own,
                                        _ => Pick::Inherit,
                                    },
                                });
                            }
                        }
                        7 => self.edit(EditTarget::Common, config.channels.join(" ")),
                        10 => self.edit(EditTarget::Temporary, String::new()),
                        11 => {
                            self.overlay = Some(Overlay::Settings {
                                index: 0,
                                editors: vec![
                                    config.max_channels.to_string(),
                                    config.poll_interval_seconds.to_string(),
                                    config.watch_interval_seconds.to_string(),
                                    config.claim_bonus.to_string(),
                                    config.client_id.clone(),
                                    config.oauth_scopes.join(" "),
                                ]
                                .into_iter()
                                .map(Editor::new)
                                .collect(),
                            })
                        }
                        12 => self.edit(EditTarget::Config, String::new()),
                        13 => return Some(Action::ToggleLog),
                        14 => self.overlay = Some(Overlay::Help),
                        15 => return Some(Action::Quit),
                        16 => return Some(Action::PlainLog),
                        _ => return Some(Action::Reload),
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
            Some(Overlay::Settings { index, editors }) => match key.code {
                KeyCode::Tab | KeyCode::Down => *index = (*index + 1) % editors.len(),
                KeyCode::BackTab | KeyCode::Up => {
                    *index = (*index + editors.len() - 1) % editors.len()
                }
                KeyCode::Enter => {
                    return Some(Action::SaveSettings(
                        editors.iter().map(|e| e.text.clone()).collect(),
                    ));
                }
                _ => editors[*index].key(key),
            },
            Some(Overlay::Help) => {
                if key.code == KeyCode::Enter {
                    self.overlay = None;
                }
            }
            Some(Overlay::Login { .. }) | Some(Overlay::Loading) => {}
            None => match key.code {
                KeyCode::Char('m') | KeyCode::F(1) => self.menu(),
                KeyCode::Char('s') => return Some(Action::Start),
                KeyCode::Char('x') => return Some(Action::Stop),
                KeyCode::Char('a') => return Some(Action::Login(None)),
                KeyCode::Char('c') => self.edit(EditTarget::Common, config.channels.join(" ")),
                KeyCode::Char('l') => return Some(Action::ToggleLog),
                KeyCode::Char('q') => return Some(Action::Quit),
                _ => {}
            },
        }
        None
    }
}

pub fn settings(config: &Config, values: &[String]) -> anyhow::Result<Config> {
    use anyhow::{Context, ensure};
    ensure!(values.len() == 6, "Не хватает полей настроек");
    crate::config::SettingsPatch {
        max_channels: Some(
            values[0]
                .trim()
                .parse()
                .context("Укажите количество каналов числом")?,
        ),
        poll_interval_seconds: Some(
            values[1]
                .trim()
                .parse()
                .context("Укажите интервал опроса числом")?,
        ),
        watch_interval_seconds: Some(
            values[2]
                .trim()
                .parse()
                .context("Укажите интервал просмотра числом")?,
        ),
        claim_bonus: Some(
            values[3]
                .trim()
                .parse()
                .context("Для бонусов укажите true или false")?,
        ),
        client_id: Some(values[4].clone()),
        oauth_scopes: Some(values[5].clone()),
    }
    .apply(config)
}

pub fn config_path(text: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!text.trim().is_empty(), "Укажите путь к конфигурации");
    Ok(PathBuf::from(text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
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
