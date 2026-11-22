# Changelog

**English** | [Русский](CHANGELOG.ru.md)

## 0.3.0 (2026-11-22)

- Automatic stable updates from GitHub at startup and every 24 hours, with
  SHA-256 verification and platform-specific archives. Updated instances restart
  with active accounts and channels; Linux keeps the systemd service PID.
- `update` and `update --check`, matching menu actions, and an `auto_update`
  setting enabled by default. Failed checks keep farming and retry after six hours.
- Show the full sign-in URL in a highlighted field; `F2` copies it in the
  interface and `login`, using the desktop or terminal clipboard.
- Replace device-code authorization with native token sign-in in `login` and
  the interface. Tokens and redirect URLs use hidden input; `login --stdin`
  supports scripts. Account names and client IDs come from Twitch validation.
- Read each account's campaign links by campaign ID, allowing linked rewards
  to start before the first watch minute and keeping unlinked games excluded.
  Sessions without access to these data request a new `login`.
- Remove obsolete Client ID and OAuth scope controls; legacy configuration keys
  are ignored. No browser dependency is added to the program.

## 0.2.1 (2026-11-22)

- Repeated Drops use each account and reward's viewing window from Twitch.
  A granted daily reward frees the slot until `nextEligibleAt` or
  `currentWindow.expiresAt`; missing reset times are shown without guessing a date.
- Confirmed grants exclude completed single rewards and fully earned repeats
  from automatic viewing.
- Regression coverage for independent account/reward windows, missing and expired
  reset times, and different watch requirements, including 90-minute daily rewards.

## 0.2.0 (2026-11-22)

- `UPDATED` includes Drops polling for channels without a channel-points balance.
- Watch-based Drops, including daily rewards, server-reported progress, and
  automatic claims confirmed by Twitch.
- Available campaigns in the interface and `drops` command, with account-specific
  eligibility checks. Optional `auto_drops` finds participating live channels.
- Points-first farming by default, configurable Drops-first priority, and mixed
  points/Drops viewing within the total limit of 1-2 streams per account.
  At most one stream is selected for Drops; a stream earning both uses one slot.
- Grouped menu, keyboard switches for basic settings, advanced fields under F2,
  and shortcuts for Drops, settings, and help.
- Higher-contrast channel states and log timestamps; scrollable help and reward details.
- Drops browser filters eligible rewards by default; Tab shows all campaigns,
  including blocked and completed rewards, with details for the selected reward.
- Fewer catalog copies and repeated log entries. Points-first farming defers
  automatic campaign scans while configured points channels fill every slot.
- Changing the automatic channel keeps configured chats connected. Shared claim
  handling and smaller API models remove duplicated work and unused fields.

Existing configuration and sessions remain compatible. Subscription rewards,
account linking, and manual reward choices remain on Twitch. Twitch controls
credited minutes and rewards; concurrent Drops campaigns are not scheduled.

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
