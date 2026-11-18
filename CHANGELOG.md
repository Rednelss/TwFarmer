# Changelog

**English** | [Русский](CHANGELOG.ru.md)

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
