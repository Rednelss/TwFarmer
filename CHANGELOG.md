# Changelog

**English** | [Русский](CHANGELOG.ru.md)

## 0.1.1 (2026-11-22)

- Join Twitch chat for all configured channels, with TLS, keepalive,
  reconnection, and refreshed credentials.
- Chat state and errors in the dashboard; a `chat_presence` setting available
  through both the interface and commands.
- A rejected channel does not disconnect other chats. Stopping farming or
  checking balances closes the chat connection.

Bot loyalty points depend on the broadcaster's settings. Chat presence is
enabled by default; the dashboard balance remains Twitch channel points.

## 0.1.0 (2026-11-19)

First public release of TwFarmer.

- Twitch device-code sign-in, saved sessions, and automatic session refresh.
- Multiple accounts named by Twitch login, each farming independently.
- Shared and personal channel lists, saved account selection, and temporary lists.
- Updating terminal interface with menus, balances, bonuses, states, and logs.
- State-based sorting with farming channels at the top and offline channels last.
- Commands for configuration and control of a running process.
- Up to two concurrent channels per account, automatic bonus claims, and retries.
- `daemon` for prepared sessions and a systemd user unit.
- English and Russian interface, messages, errors, and help; saved language
  selection and a temporary `--lang` override.

Twitch decides whether to award points. Internal requests can change; a
successful stream request does not guarantee points.
