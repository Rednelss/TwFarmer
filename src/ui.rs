use crate::{
    accounts::Profile,
    config::Config,
    control::{Controls, EditTarget, Overlay, menu_items, settings_fields},
    model::{AccountStatus, ChannelStatus, ChatStatus, Dashboard, duration_text},
};
use anyhow::{Context, Result};
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Tabs, Wrap,
    },
};
use std::{
    collections::HashSet,
    io::{self, Stdout},
    path::Path,
};
use tokio::time::Instant;

pub struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}
impl Screen {
    pub fn new() -> Result<Self> {
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout())).context(tr!(
            "Could not create the dashboard",
            "Не удалось создать панель"
        ))?;
        let mut screen = Self { terminal };
        enable_raw_mode().context(tr!(
            "Could not enable terminal raw mode",
            "Не удалось включить режим терминала"
        ))?;
        execute!(
            screen.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste
        )
        .context(tr!(
            "Could not open the dashboard screen",
            "Не удалось открыть экран панели"
        ))?;
        screen.terminal.hide_cursor()?;
        Ok(screen)
    }
    pub fn draw_shell(
        &mut self,
        dashboard: &Dashboard,
        state: &mut ViewState,
        shell: ShellView<'_>,
    ) -> Result<()> {
        self.terminal
            .draw(|frame| render_shell(frame, dashboard, state, &shell))
            .context(tr!(
                "Could not update the interface",
                "Не удалось обновить интерфейс"
            ))?;
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

pub struct ViewState {
    pub table: TableState,
    pub filter: Option<usize>,
    selected_key: Option<(String, String)>,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            table: TableState::default().with_selected(0),
            filter: None,
            selected_key: None,
        }
    }
}
impl ViewState {
    pub fn sync_selection(&mut self, dashboard: &Dashboard) {
        let rows = dashboard.rows(self.filter);
        if rows.is_empty() {
            self.table.select(None);
            self.selected_key = None;
            return;
        }
        let index = self
            .selected_key
            .as_ref()
            .and_then(|(name, channel)| {
                rows.iter()
                    .position(|(a, c)| a.name == *name && c.name == *channel)
            })
            .unwrap_or_else(|| self.table.selected().unwrap_or(0).min(rows.len() - 1));
        self.table.select(Some(index));
        self.selected_key = Some((rows[index].0.name.clone(), rows[index].1.name.clone()));
    }
    pub fn move_selection(&mut self, dashboard: &Dashboard, delta: isize) {
        self.sync_selection(dashboard);
        let rows = dashboard.rows(self.filter);
        if rows.is_empty() {
            return;
        }
        let index = self
            .table
            .selected()
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(rows.len() - 1);
        self.table.select(Some(index));
        self.selected_key = Some((rows[index].0.name.clone(), rows[index].1.name.clone()));
    }

    pub fn next_account(&mut self, dashboard: &Dashboard) {
        self.filter = match self.filter {
            None if !dashboard.accounts.is_empty() => Some(0),
            Some(n) if n + 1 < dashboard.accounts.len() => Some(n + 1),
            _ => None,
        };
        self.table = TableState::default().with_selected(0);
        self.selected_key = None;
        self.sync_selection(dashboard);
    }
}

fn status_style(status: &ChannelStatus) -> Style {
    let color = match status {
        ChannelStatus::Watching => Color::Green,
        ChannelStatus::Error => Color::Red,
        ChannelStatus::Retry(_) => Color::Yellow,
        ChannelStatus::Starting | ChannelStatus::Checking => Color::Cyan,
        ChannelStatus::Online | ChannelStatus::Queued => Color::White,
        ChannelStatus::Idle | ChannelStatus::Offline | ChannelStatus::Disabled => Color::DarkGray,
    };
    Style::default().fg(color)
}

pub fn render(frame: &mut Frame, dashboard: &Dashboard, state: &mut ViewState) {
    let area = frame.area();
    let log_height = if area.height >= 28 {
        7
    } else if area.height >= 20 {
        5
    } else {
        3
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(if area.height >= 24 {
                6
            } else if area.height >= 20 {
                5
            } else {
                3
            }),
            Constraint::Length(log_height),
            Constraint::Length(1),
        ])
        .split(area);
    let running = dashboard
        .accounts
        .iter()
        .filter(|a| matches!(a.status, AccountStatus::Running | AccountStatus::Waiting))
        .count();
    let watching = dashboard
        .rows(None)
        .iter()
        .filter(|(_, c)| c.status == ChannelStatus::Watching)
        .count();
    let header = Line::from(vec![
        Span::styled(
            " TwFarmer ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(tf!(
            " {}  •  Accounts {running}/{}  •  Farming {watching}  •  ",
            " {}  •  Аккаунты {running}/{}  •  Фарм {watching}  •  ",
            duration_text(dashboard.started.elapsed()),
            dashboard.accounts.len()
        )),
        Span::styled(
            tf!("{:+} points", "{:+} баллов", dashboard.gain()),
            Style::default().fg(Color::Green),
        ),
        Span::raw(tf!(
            "  •  Bonuses {}",
            "  •  Бонусов {}",
            dashboard.bonuses()
        )),
    ]);
    frame.render_widget(
        Paragraph::new(header).block(Block::default().borders(Borders::ALL)),
        chunks[0],
    );
    let tabs = std::iter::once(Line::from(tr!("All accounts", "Все аккаунты")))
        .chain(dashboard.accounts.iter().map(|a| {
            let prefix = if a.status == AccountStatus::Error {
                "! "
            } else {
                ""
            };
            Line::from(format!("{prefix}{}", a.name))
        }))
        .collect::<Vec<_>>();
    frame.render_widget(
        Tabs::new(tabs)
            .select(state.filter.map_or(0, |n| n + 1))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .title(tr!(" Twitch accounts ", " Аккаунты Twitch "))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
    let wide = area.width >= 110;
    let medium = area.width >= 80;
    let chat_column = area.width >= 130;
    let mut headers = if wide {
        vec![
            tr!("ACCOUNT", "АККАУНТ"),
            tr!("CHANNEL", "КАНАЛ"),
            tr!("STATE", "СОСТОЯНИЕ"),
            tr!("BALANCE", "БАЛАНС"),
            tr!("GAIN", "ПРИРОСТ"),
            tr!("BONUSES", "БОНУСЫ"),
            tr!("ACTIVE", "АКТИВНО"),
            tr!("UPDATED", "ОБНОВЛ."),
        ]
    } else if medium {
        vec![
            tr!("ACCOUNT", "АККАУНТ"),
            tr!("CHANNEL", "КАНАЛ"),
            tr!("STATE", "СОСТОЯНИЕ"),
            tr!("BALANCE", "БАЛАНС"),
            tr!("GAIN", "ПРИРОСТ"),
            tr!("BONUSES", "БОНУСЫ"),
        ]
    } else {
        vec![
            tr!("ACCOUNT", "АККАУНТ"),
            tr!("CHANNEL", "КАНАЛ"),
            tr!("STATE", "СОСТОЯНИЕ"),
            tr!("BALANCE", "БАЛАНС"),
            "+",
        ]
    };
    if chat_column {
        headers.push(tr!("CHAT", "ЧАТ"));
    }
    let widths: Vec<_> = (if chat_column {
        vec![16, 17, 15, 9, 9, 6, 9, 7, 12]
    } else if wide {
        vec![18, 19, 17, 11, 11, 7, 10, 7]
    } else if medium {
        vec![19, 23, 23, 14, 13, 8]
    } else {
        vec![22, 25, 25, 14, 14]
    })
    .into_iter()
    .map(Constraint::Percentage)
    .collect();
    state.sync_selection(dashboard);
    let entries = dashboard.rows(state.filter);
    if state.table.selected().is_some_and(|n| n >= entries.len()) {
        state
            .table
            .select(if entries.is_empty() { None } else { Some(0) });
    }
    let rows = entries.iter().map(|(account, channel)| {
        let name = account
            .login
            .clone()
            .unwrap_or_else(|| account.name.clone());
        let gain = channel
            .balance
            .map_or_else(|| "-".into(), |_| format!("{:+}", channel.gain()));
        let mut cells = vec![
            Cell::from(name),
            Cell::from(channel.name.clone()),
            Cell::from(channel.status.label()).style(status_style(&channel.status)),
            Cell::from(
                channel
                    .balance
                    .map_or_else(|| "-".into(), |n| n.to_string()),
            ),
            Cell::from(gain).style(Style::default().fg(if channel.gain() < 0 {
                Color::Yellow
            } else {
                Color::Green
            })),
        ];
        if medium {
            cells.push(Cell::from(channel.bonuses.to_string()));
        }
        if wide {
            cells.push(Cell::from(duration_text(channel.active_time)));
            cells.push(Cell::from(channel.last_update.map_or_else(
                || "-".into(),
                |at| {
                    tf!(
                        "{}s",
                        "{}с",
                        Instant::now().saturating_duration_since(at).as_secs()
                    )
                },
            )));
        }
        if chat_column {
            let color = match channel.chat_status {
                ChatStatus::Joined => Color::Green,
                ChatStatus::Connecting => Color::Cyan,
                ChatStatus::Retry(_) => Color::Yellow,
                ChatStatus::Error => Color::Red,
                ChatStatus::Disabled => Color::Gray,
            };
            cells.push(Cell::from(channel.chat_status.label()).style(Style::default().fg(color)));
        }
        Row::new(cells)
    });
    frame.render_stateful_widget(
        Table::new(rows, widths)
            .header(
                Row::new(headers)
                    .style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .bottom_margin(1),
            )
            .column_spacing(1)
            .row_highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("› ")
            .block(
                Block::default()
                    .title(tr!(
                        " Channels • up to 2 concurrent channels per account ",
                        " Каналы • до 2 одновременно на аккаунт "
                    ))
                    .borders(Borders::ALL),
            ),
        chunks[2],
        &mut state.table,
    );
    let detail = if let Some((account, channel)) =
        state.table.selected().and_then(|n| entries.get(n))
    {
        let last = channel.last_success.map_or_else(
            || tr!("never", "ещё не было").into(),
            |at| {
                tf!(
                    "{}s ago",
                    "{}с назад",
                    Instant::now().saturating_duration_since(at).as_secs()
                )
            },
        );
        let message = channel.error.as_ref().or(account.error.as_ref()).cloned().unwrap_or_else(|| {
            match channel.status {
                ChannelStatus::Offline => tr!("Waiting for the next stream.", "Ожидание следующего эфира.").into(),
                ChannelStatus::Queued => tr!("Waiting for a free slot; channel order sets priority.", "Ожидание свободного места; приоритет задан порядком каналов.").into(),
                ChannelStatus::Disabled => tr!("Channel points are disabled for this channel.", "На канале выключены баллы.").into(),
                _ => tr!("Twitch provides the balance. The first points usually arrive after a few minutes.", "Баланс приходит из Twitch. Первые баллы обычно появляются через несколько минут.").into(),
            }
        });
        let chat = channel.chat_status.label();
        let chat_error = channel
            .chat_error
            .as_ref()
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        tf!(
            "{} • {} • last request: {last}\nChat: {chat}{chat_error}\n{message}",
            "{} • {} • последнее обращение: {last}\nЧат: {chat}{chat_error}\n{message}",
            account.login.as_deref().unwrap_or(&account.name),
            channel.name
        )
    } else if let Some(account) = state.filter.and_then(|n| dashboard.accounts.get(n)) {
        account.error.clone().unwrap_or_else(|| {
            tr!(
                "No channels are configured for this account.",
                "Для аккаунта не настроены каналы."
            )
            .into()
        })
    } else {
        tr!(
            "↑/↓ to select a channel. Tab to switch accounts.",
            "↑/↓: выбор канала. Tab: смена аккаунта."
        )
        .into()
    };
    frame.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(tr!(
                    " Selected channel details ",
                    " Подробности выбранного канала "
                ))
                .borders(Borders::ALL),
        ),
        chunks[3],
    );
    let count = chunks[4].height.saturating_sub(2) as usize;
    let logs = dashboard
        .logs
        .iter()
        .skip(dashboard.logs.len().saturating_sub(count))
        .map(|log| {
            let elapsed = log.at.saturating_duration_since(dashboard.started);
            Line::from(vec![
                Span::styled(
                    format!("{} {}: ", duration_text(elapsed), log.profile),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    log.message.clone(),
                    Style::default().fg(if log.error {
                        Color::Yellow
                    } else {
                        Color::White
                    }),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(logs).block(
            Block::default()
                .title(tr!(" Events ", " События "))
                .borders(Borders::ALL),
        ),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(tr!(
            " ↑/↓ or j/k: channels  •  Tab: accounts  •  PgUp/PgDn: scroll  •  q / Ctrl+C: quit",
            " ↑/↓ или j/k: каналы  •  Tab: аккаунты  •  PgUp/PgDn: прокрутка  •  q / Ctrl+C: выход"
        ))
        .style(Style::default().fg(Color::DarkGray)),
        chunks[5],
    );
}

pub struct ShellView<'a> {
    pub controls: &'a Controls,
    pub config: &'a Config,
    pub profiles: &'a [Profile],
    pub selected: &'a HashSet<String>,
    pub path: &'a Path,
    pub temporary: &'a [String],
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn input_line(editor: &crate::control::Editor, width: usize) -> String {
    let chars: Vec<_> = editor.text.chars().collect();
    let start = editor.cursor.saturating_sub(width.saturating_sub(3));
    let end = (start + width.saturating_sub(1)).min(chars.len());
    let before: String = chars[start..editor.cursor.min(end)].iter().collect();
    let after: String = chars[editor.cursor.min(end)..end].iter().collect();
    format!("{before}▏{after}")
}

pub fn render_shell(
    frame: &mut Frame,
    dashboard: &Dashboard,
    state: &mut ViewState,
    shell: &ShellView<'_>,
) {
    render(frame, dashboard, state);
    let area = frame.area();
    if shell.controls.log_only {
        let logs = dashboard
            .logs
            .iter()
            .rev()
            .take(area.height.saturating_sub(6) as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|log| Line::from(format!("{}: {}", log.profile, log.message)))
            .collect::<Vec<_>>();
        let log_area = Rect::new(
            area.x,
            area.y + 3,
            area.width,
            area.height.saturating_sub(6),
        );
        frame.render_widget(Clear, log_area);
        frame.render_widget(
            Paragraph::new(logs).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title(tr!(
                        " Log • l to return to the table ",
                        " Журнал • l: вернуться к таблице "
                    ))
                    .borders(Borders::ALL),
            ),
            log_area,
        );
    }
    let footer = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(3),
        area.width,
        3.min(area.height),
    );
    frame.render_widget(Clear, footer);
    let source = if shell.temporary.is_empty() {
        tr!("saved lists", "сохранённые списки").into()
    } else {
        tf!("temporary: {}", "временные: {}", shell.temporary.join(", "))
    };
    frame.render_widget(Paragraph::new(vec![
        Line::from(Span::styled(&shell.controls.notice,Style::default().fg(Color::Yellow))),
        Line::from(tf!("Selected {}/{} • {} • {}", "Выбрано {}/{} • {} • {}",shell.selected.len(),shell.profiles.len(),source,shell.path.display())),
        Line::from(Span::styled(tr!(" m / F1: menu • s: start • x: stop • a: login • c: channels • Tab: account • q: quit", " m / F1: меню • s: старт • x: стоп • a: вход • c: каналы • Tab: аккаунт • q: выход"),
            Style::default().fg(Color::Cyan))),
    ]),footer);

    let Some(overlay) = &shell.controls.overlay else {
        return;
    };
    let height = match overlay {
        Overlay::Menu(_) => 23,
        Overlay::Help => 24,
        Overlay::Settings { .. } => 22,
        Overlay::Accounts { .. } | Overlay::Pick { .. } => 20,
        Overlay::Login { .. } => 14,
        _ => 12,
    };
    let rect = popup(area, 100, height);
    frame.render_widget(Clear, rect);
    let title = match overlay {
        Overlay::Language(_) => "Language / Язык",
        Overlay::Menu(_) => tr!("Menu", "Меню"),
        Overlay::Accounts { .. } => tr!("Accounts to start", "Аккаунты для запуска"),
        Overlay::Pick { .. } => tr!("Select an account", "Выберите аккаунт"),
        Overlay::Confirm { .. } => tr!("Remove account", "Удаление аккаунта"),
        Overlay::Edit { target, .. } => match target {
            EditTarget::Common => tr!("Shared channels", "Общие каналы"),
            EditTarget::Own(_) => tr!("Personal channels", "Личные каналы"),
            EditTarget::Temporary => tr!(
                "Channels for this run only",
                "Каналы только на текущий запуск"
            ),
            EditTarget::Config => tr!(
                "Open / create a configuration",
                "Открыть / создать конфигурацию"
            ),
        },
        Overlay::Settings { .. } => tr!("Farming settings", "Настройки фарма"),
        Overlay::Login { .. } => tr!("Twitch sign-in", "Вход в Twitch"),
        Overlay::Loading => tr!("Loading accounts", "Загрузка аккаунтов"),
        Overlay::Help => tr!("Help", "Помощь"),
    };
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(inner);
    let highlight = Style::default().bg(Color::DarkGray).fg(Color::Cyan);
    let help = match overlay {
        Overlay::Language(_) => tr!(
            "↑/↓: language • Enter: save • Esc: cancel",
            "↑/↓: язык • Enter: сохранить • Esc: отмена"
        ),
        Overlay::Accounts { .. } => {
            tr!(
                "↑/↓: account • Space: select • a: all • n: none • Enter: save • t: this run • Esc: cancel",
                "↑/↓: аккаунт • Пробел: выбрать • a: все • n: никто • Enter: сохранить • t: на этот запуск • Esc: отмена"
            )
        }
        Overlay::Menu(_) | Overlay::Pick { .. } => {
            tr!(
                "↑/↓: select • Enter: run • Esc: close",
                "↑/↓: выбрать • Enter: выполнить • Esc: закрыть"
            )
        }
        Overlay::Settings { .. } => {
            tr!(
                "Tab / ↑↓: field • Ctrl+U: clear • Enter: save • Esc: cancel",
                "Tab / ↑↓: поле • Ctrl+U: очистить • Enter: сохранить • Esc: отмена"
            )
        }
        Overlay::Edit { .. } => {
            tr!(
                "←/→: cursor • Ctrl+U: clear • Enter: save • Esc: cancel",
                "←/→: курсор • Ctrl+U: очистить • Enter: сохранить • Esc: отмена"
            )
        }
        Overlay::Login { .. } | Overlay::Loading => {
            tr!(
                "Esc to cancel. Other accounts continue running.",
                "Esc для отмены. Остальные аккаунты продолжают работать."
            )
        }
        Overlay::Confirm { .. } => tr!(
            "←/→ or Tab: select • Enter: confirm • Esc: cancel",
            "←/→ или Tab: выбор • Enter: подтвердить • Esc: отменить"
        ),
        Overlay::Help => tr!("Enter / Esc to close", "Enter / Esc для закрытия"),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                &shell.controls.notice,
                Style::default().fg(Color::Yellow),
            )),
            Line::from(help),
        ])
        .wrap(Wrap { trim: false }),
        chunks[1],
    );
    match overlay {
        Overlay::Language(index) => {
            frame.render_stateful_widget(
                List::new(["English", "Русский"])
                    .highlight_style(highlight)
                    .highlight_symbol("› "),
                chunks[0],
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Overlay::Menu(index) => {
            let items = menu_items().into_iter().map(ListItem::new);
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(highlight)
                    .highlight_symbol("› "),
                chunks[0],
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Overlay::Accounts { index, chosen } => {
            let items = shell.profiles.iter().map(|p| {
                let source = if shell.config.account_channels.contains_key(&p.config_key) {
                    tr!("personal", "свои")
                } else {
                    tr!("shared", "общие")
                };
                ListItem::new(format!(
                    "[{}] {} • {source}: {}",
                    if chosen.contains(&p.name) { "x" } else { " " },
                    p.name,
                    shell.config.channels_for(&p.config_key).join(", ")
                ))
            });
            frame.render_stateful_widget(
                List::new(items).highlight_style(highlight),
                chunks[0],
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Overlay::Pick { index, .. } => {
            frame.render_stateful_widget(
                List::new(shell.profiles.iter().map(|p| ListItem::new(p.name.clone())))
                    .highlight_style(highlight)
                    .highlight_symbol("› "),
                chunks[0],
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Overlay::Confirm { name, yes } => {
            frame.render_widget(
                Paragraph::new(tf!(
                    "Remove the local session for account {name}?\n\n{}",
                    "Удалить локальную сессию аккаунта {name}?\n\n{}",
                    if *yes {
                        tr!("No    [ Yes, remove ]", "Нет    [ Да, удалить ]")
                    } else {
                        tr!("[ No ]    Yes, remove", "[ Нет ]    Да, удалить")
                    }
                ))
                .wrap(Wrap { trim: false }),
                chunks[0],
            );
        }
        Overlay::Edit { target, editor } => {
            let description=match target {
                EditTarget::Config=>tf!("Settings file path. A new file will be created.\nCurrent: {}", "Путь к файлу настроек. Новый файл будет создан.\nСейчас: {}",shell.path.display()),
                EditTarget::Own(name)=>tf!("Account: {name}. Logins or URLs separated by spaces/commas.\nEmpty input uses the shared list.", "Аккаунт: {name}. Логины или ссылки через пробел/запятую.\nПустой ввод возвращает общий список."),
                EditTarget::Temporary=>tr!("Logins or URLs separated by spaces/commas for all selected accounts.\nEmpty input restores the saved lists. This list is not saved to disk.", "Логины или ссылки через пробел/запятую для всех выбранных аккаунтов.\nПустой ввод возвращает сохранённые списки. На диск не записывается.").into(),
                EditTarget::Common=>tr!("Twitch logins or URLs separated by spaces/commas.\nAccounts without personal channels use the shared list.", "Логины или ссылки Twitch через пробел/запятую.\nОбщий список используется аккаунтами без личного списка.").into(),
            };
            let editor_areas =
                Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(chunks[0]);
            frame.render_widget(
                Paragraph::new(description).wrap(Wrap { trim: false }),
                editor_areas[0],
            );
            frame.render_widget(
                Paragraph::new(input_line(editor, editor_areas[1].width as usize))
                    .style(Style::default().fg(Color::Cyan)),
                editor_areas[1],
            );
        }
        Overlay::Settings { index, editors } => {
            let items = editors.iter().enumerate().map(|(n, editor)| {
                ListItem::new(vec![
                    Line::from(settings_fields()[n]),
                    Line::from(if n == *index {
                        input_line(editor, chunks[0].width as usize)
                    } else {
                        editor.text.clone()
                    }),
                ])
            });
            frame.render_stateful_widget(
                List::new(items).highlight_style(highlight),
                chunks[0],
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Overlay::Login { uri, code } => {
            let text = if code.is_empty() {
                tr!(
                    "Requesting a login code from Twitch…",
                    "Запрашиваю код входа у Twitch…"
                )
                .into()
            } else {
                tf!(
                    "Open in your browser:\n{uri}\n\nCode: {code}\n\nSelect the intended Twitch account and confirm sign-in.\nIts name appears automatically after confirmation.",
                    "Откройте в браузере:\n{uri}\n\nКод: {code}\n\nВыберите нужный Twitch-аккаунт и подтвердите вход.\nИмя появится автоматически после подтверждения."
                )
            };
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), chunks[0]);
        }
        Overlay::Loading => {
            frame.render_widget(
                Paragraph::new(tr!(
                    "Reading settings and saved accounts…",
                    "Читаю настройки и сохранённые аккаунты…"
                ))
                .wrap(Wrap { trim: false }),
                chunks[0],
            );
        }
        Overlay::Help => {
            frame.render_widget(Paragraph::new(
                tf!("TwFarmer {}. Open the menu with m / F1.\n\nAccount names come from Twitch logins at sign-in.\n\nAccounts without personal channels use the shared list. Channel order sets priority.\n\nTemporary channels apply to all selected accounts until cleared or the program exits.\n\nAccount selection and settings are saved. Changes during farming apply automatically.\n\n↑/↓ or j/k: channel; Tab: account filter; PgUp/PgDn: scroll.\n\nCheck balances in the menu. Start farming to watch streams and claim bonuses.\n\nq / Ctrl+C to quit.", "TwFarmer {}. Откройте меню через m / F1.\n\nПри входе имя аккаунта определяется по логину Twitch.\n\nАккаунты без личного списка используют общие каналы. Порядок каналов задаёт приоритет.\n\nВременный список действует на все выбранные аккаунты до его очистки или выхода.\n\nВыбор аккаунтов и настройки сохраняются. Изменения во время фарма применяются автоматически.\n\n↑/↓ или j/k: канал; Tab: фильтр аккаунтов; PgUp/PgDn: прокрутка.\n\nПроверяйте балансы через меню. Запустите фарм для просмотра и сбора бонусов.\n\nq / Ctrl+C для выхода.", env!("CARGO_PKG_VERSION"))
            ).wrap(Wrap{trim:false}),chunks[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChannelView, Event};
    use ratatui::backend::TestBackend;

    #[test]
    fn chat_state_and_errors_render_in_narrow_and_wide_terminals() {
        for width in [80, 140] {
            let mut model = Dashboard::new(vec![("account".into(), None, vec!["channel".into()])]);
            model.accounts[0].channels[0].chat_status = ChatStatus::Joined;
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            let mut view = ViewState::default();
            terminal
                .draw(|frame| render(frame, &model, &mut view))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("Chat: Joined"));
            if width >= 130 {
                assert!(text.contains("CHAT"));
            }
            model.accounts[0].channels[0].chat_status = ChatStatus::Error;
            model.accounts[0].channels[0].chat_error = Some("chat unavailable".into());
            terminal
                .draw(|frame| render(frame, &model, &mut view))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("Chat: Error: chat unavailable"));
        }
    }

    #[test]
    fn selection_follows_the_channel_when_state_sorting_moves_it() {
        let mut model = Dashboard::new(vec![(
            "user".into(),
            None,
            vec!["first".into(), "second".into()],
        )]);
        let mut view = ViewState::default();
        view.sync_selection(&model);
        assert_eq!(view.table.selected(), Some(0));
        model.accounts[0].channels[1].status = ChannelStatus::Watching;
        view.sync_selection(&model);
        assert_eq!(view.table.selected(), Some(1));
        view.move_selection(&model, -1);
        assert_eq!(view.selected_key.as_ref().unwrap().1, "second");
        model.accounts[0].channels[1].status = ChannelStatus::Offline;
        view.sync_selection(&model);
        assert_eq!(view.table.selected(), Some(1));
    }

    #[test]
    fn menus_and_forms_work_at_small_sizes_without_showing_aliases() {
        let dashboard = Dashboard::new(vec![(
            "twitch_user".into(),
            Some("twitch_user".into()),
            vec!["channel".into()],
        )]);
        let profiles = vec![Profile {
            name: "twitch_user".into(),
            login: Some("twitch_user".into()),
            path: std::path::PathBuf::from("/tmp/session"),
            config_key: "twitch_user".into(),
        }];
        let config = Config::default();
        let selected = HashSet::from(["twitch_user".into()]);
        let mut controls = Controls::default();
        for (width, height) in [(140, 32), (40, 12), (10, 5)] {
            for overlay in [
                Overlay::Menu(15),
                Overlay::Accounts {
                    index: 0,
                    chosen: selected.clone(),
                },
                Overlay::Pick {
                    kind: crate::control::Pick::Own,
                    index: 0,
                },
                Overlay::Confirm {
                    name: "twitch_user".into(),
                    yes: false,
                },
                Overlay::Edit {
                    target: EditTarget::Common,
                    editor: crate::control::Editor::new("channel".into()),
                },
                Overlay::Settings {
                    index: 7,
                    editors: vec![crate::control::Editor::new("value".into()); 8],
                },
                Overlay::Login {
                    uri: "https://www.twitch.tv/activate".into(),
                    code: "TESTCODE".into(),
                },
                Overlay::Loading,
                Overlay::Help,
            ] {
                let editor = matches!(overlay, Overlay::Edit { .. });
                controls.overlay = Some(overlay);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        render_shell(
                            frame,
                            &dashboard,
                            &mut ViewState::default(),
                            &ShellView {
                                controls: &controls,
                                config: &config,
                                profiles: &profiles,
                                selected: &selected,
                                path: Path::new("twfarmer.toml"),
                                temporary: &[],
                            },
                        )
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let text = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if editor && width == 40 {
                    assert!(text.contains("▏"), "Input cursor is hidden");
                }
                assert!(!text.contains("default"));
            }
        }
    }

    #[test]
    fn dashboard_renders_accounts_balances_and_a_failed_profile() {
        let mut dashboard = Dashboard::new(vec![
            (
                "main".into(),
                Some("user-main".into()),
                vec!["channel_a".into()],
            ),
            (
                "alt".into(),
                Some("user-alt".into()),
                vec!["channel_b".into()],
            ),
        ]);
        let mut view = ChannelView::new("channel_a".into());
        view.set_balance(100);
        view.set_balance(150);
        view.status = ChannelStatus::Watching;
        dashboard.apply(Event::Channel {
            profile: "main".into(),
            view,
        });
        dashboard.apply(Event::Account {
            profile: "alt".into(),
            login: None,
            status: AccountStatus::Error,
            error: Some("Session expired".into()),
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|f| render(f, &dashboard, &mut ViewState::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..30)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("TwFarmer"));
        assert!(!text.contains("main / user-main"));
        assert!(text.contains("user-main"));
        assert!(text.contains("channel_a"));
        assert!(text.contains("channel_b"));
        assert!(text.contains("150"));
        assert!(text.contains("+50"));
        assert!(text.contains("Error"));
    }

    #[test]
    fn small_terminal_and_empty_account_can_be_rendered_and_filtered() {
        let dashboard = Dashboard::new(vec![("main".into(), None, vec![])]);
        let mut state = ViewState::default();
        state.next_account(&dashboard);
        state.move_selection(&dashboard, 1);
        assert_eq!(state.filter, Some(0));
        assert!(state.table.selected().is_none());
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| render(f, &dashboard, &mut state))
            .unwrap();
        state.next_account(&dashboard);
        assert_eq!(state.filter, None);
    }
}
