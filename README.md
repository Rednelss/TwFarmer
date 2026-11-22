# TwFarmer

**English** | [Русский](README.ru.md)

A Rust terminal application for Twitch channel points farming and claiming
available bonuses with multiple accounts. Accounts use their Twitch logins;
you do not need to invent separate profile names.

## Installation

Download an archive for your system from
[GitHub Releases](https://github.com/Rednelss/TwFarmer/releases).
Extract it and run `twfarmer` (`twfarmer.exe` on Windows) from the extracted folder.
Release builds cover Linux x86_64, Windows x86_64, and macOS Apple Silicon.
The GitHub Actions Linux build requires glibc 2.35 or newer.
Compare the archive's SHA-256 with `SHA256SUMS` from the release. On Linux:

```bash
sha256sum --ignore-missing --check SHA256SUMS
```

To build from source, install Rust 1.89+ and Cargo:

```bash
git clone https://github.com/Rednelss/TwFarmer.git
cd TwFarmer
cargo build --release --locked
cd target/release
./twfarmer
```

The management menu opens. Complete the first setup in the interface: add an
account, set shared channels, select accounts, and start farming.
Sign-in shows a URL and a device code. Confirm it on the Twitch website using
the intended account. Passwords and 2FA are entered on Twitch, and the login
appears in the program automatically after confirmation.

Keep using the same working directory or `--config` path for existing settings.
Legacy session files are read automatically; personal channel lists using old
profile aliases are migrated to Twitch logins.

## Language

English is the default. Open the menu with `m` / `F1`, select **Language / Язык**,
choose **English** or **Русский**, and press `Enter`. The choice is saved in the
configuration's `language` field and takes effect immediately. Changing language
with this menu item does not restart farming.

```bash
./twfarmer language en                 # save English
./twfarmer language ru                 # save Russian
./twfarmer language                    # show the saved language
./twfarmer settings set --language ru  # the same choice through settings
./twfarmer --lang ru --help            # Russian for this command only
./twfarmer --lang en run               # English for this run only
```

`--lang` overrides the saved language without changing the configuration file.
Selecting a different language in the interface saves and applies that choice.
Command names, configuration keys, and Twitch logins are the same in both languages.
The interface, help, sign-in messages, and new log entries use the chosen language;
existing log entries keep their original text. Old configuration files without
`language` default to English.

## Terminal interface

Press `m` or `F1` to open the menu, `↑/↓` to select, `Enter` to apply, and
`Esc` to close a form or cancel sign-in. The menu provides:

- Start and stop farming, or check balances without watching or claiming bonuses.
- Select accounts: Space toggles one, `a` selects all, `n` selects none,
  `Enter` saves, and `t` applies the selection for this run only.
- Add accounts, renew sign-in, and remove local sessions.
- Edit shared channels, personal channels, or restore the shared list for an account.
- Set temporary channels for the current process.
- Edit farming settings, open a configuration, or create a new one.
- Choose the language, switch between the table and log, view help, and quit.

Channel forms accept Twitch logins or URLs separated by spaces or commas.
An empty personal list uses the shared list. An empty temporary list restores
saved lists. Forms support `←/→`, `Home/End`, `Backspace/Delete`, paste, and
`Ctrl+U` to clear input. Use `Tab` or `↑/↓` between settings fields.

Shared and personal channels, settings, and the account selection are saved.
Temporary channels apply to all selected accounts in the current process and
override their saved lists. Changing farming settings restarts selected accounts
with the new settings while preserving dashboard totals.
Checking balances stops watching. Other accounts continue running during sign-in.

Dashboard shortcuts:

- `s`: start; `x`: stop; `a`: add an account; `c`: shared channels.
- `↑/↓` or `j/k`: select a channel; `Tab`: filter by account.
- `PgUp/PgDn`: scroll; `l`: toggle the log.
- `q` or `Ctrl+C`: quit.

Channels are sorted by state: farming, starting, retrying, online, queued,
checking, stopped, errors, points disabled, and offline. Within the same state,
the configured order is preserved. Selection follows a channel when its row
moves. Watching priority always follows configuration order.

The dashboard updates every 250 ms and shows balances, balance changes for this
process, bonuses, states, and errors. Wide terminals also show active time and
the age of the balance. Channel details show the last successful watch request.
A successful stream request does not itself confirm that points were awarded.

Each account runs independently and watches up to two channels concurrently.
Failure of one account does not stop the others. When a stream ends, the next
eligible online channel is selected in priority order.

## Chat presence

During farming, each account joins Twitch chat for every channel in its current
shared, personal, or temporary list, including offline channels. The connection
uses TLS on `irc.chat.twitch.tv:6697`, answers keepalive requests, reconnects
after failures, and uses updated credentials when a session refreshes.
Chat runs independently of the video watch requests.

`chat_presence` is enabled by default. Change it in **Farming settings > Join chat**
or with a command:

```bash
./twfarmer settings set --chat-presence true
./twfarmer settings set --chat-presence false
```

Channel details show chat state and errors. Terminals at least 130 columns wide
also show a `CHAT` column. Stopping farming, checking balances, or exiting closes
the chat connection. The program sends no chat messages and does not store or
print incoming messages.

StreamElements and similar bots maintain their own loyalty balances. Whether
chat presence earns those points depends on the broadcaster's bot settings;
some rules require activity or a live stream. The dashboard balance is still
Twitch channel points. Twitch's users-in-chat list may take time to update.
If Twitch rejects IRC login, renew the account's sign-in. Tokens from some
OAuth clients require `chat:read` in `oauth_scopes` for IRC access.

## Commands

Commands support scripts and use Twitch logins as account names:

```bash
./twfarmer                                     # management menu
./twfarmer ui                                  # same interface
./twfarmer login                               # add or renew an account
./twfarmer login --account twitch_user          # renew this account
./twfarmer accounts add                        # no separate profile name
./twfarmer accounts add welovegames lizabaktt   # add with personal channels
./twfarmer accounts list
./twfarmer channels welovegames betboom_cs_ru
./twfarmer accounts channels twitch_user lizabaktt
./twfarmer accounts channels twitch_user        # restore the shared list
./twfarmer run                                 # farming and interface
./twfarmer run --account twitch_user,another_login
./twfarmer run --account twitch_user welovegames # temporary channels
./twfarmer run --no-ui                          # plain log
./twfarmer status --account twitch_user
./twfarmer accounts remove twitch_user
./twfarmer logout --account twitch_user
./twfarmer --config /absolute/path/settings.toml
```

`run` uses the saved account selection, or all saved accounts if no selection
has been saved. `--account` overrides the selection only for that run.
`run CHANNELS` overrides channel lists only for the current process.
Redirected output uses a plain log. `init CHANNELS` creates settings without
overwriting an existing file. `logout` without a login requires exactly one
saved account.

Removing a local session does not revoke Twitch access. Manage authorized
applications at https://www.twitch.tv/settings/connections.

## Menu and command reference

| Interface action | Command |
| --- | --- |
| Start farming | `run`; `start` controls an already running program |
| Start prepared sessions without the interface | `daemon` (alias: `service`) |
| Stop farming | `stop` |
| Check balances | `status --account LOGIN` |
| Save account selection, Enter | `accounts select LOGIN1 LOGIN2` |
| Clear account selection | `accounts select` |
| Select all accounts | `accounts select --all` |
| Temporary account selection, t | `run --account LOGIN` or `start --account LOGIN` |
| Add an account | `login` or `accounts add` |
| Renew sign-in | `login --account LOGIN` |
| Remove an account and its settings | `accounts remove LOGIN` or `logout --account LOGIN` |
| List accounts | `accounts list` |
| Shared channels | `channels CHANNELS` |
| Clear shared channels | `channels --clear` |
| Personal channels | `accounts channels LOGIN CHANNELS` |
| Use shared channels | `accounts channels LOGIN` |
| Temporary channels | `run CHANNELS` or `start CHANNELS` |
| Clear temporary channels | `start` without channels |
| Settings | `settings show`, `settings set OPTIONS` |
| Chat presence | `settings set --chat-presence true` or `false` |
| Open a configuration | `--config PATH ui` |
| Create a configuration, including an empty one | `--config PATH init [CHANNELS]` |
| Log instead of table | `ui --journal`, `run --journal`, `journal` |
| Restore table | `journal --enabled false` |
| Plain log | `run --no-ui` |
| Reload settings | `reload` |
| Language | `language en`, `language ru`, `settings set --language en` |
| Help / version | `--help`, `--version` |
| Close program | `quit`, Ctrl+C |

`start`, `stop`, `reload`, `journal`, and `quit` connect to the running program
using the same configuration path. From another terminal:

```bash
./twfarmer settings set --max-channels 1 --claim-bonus false
./twfarmer accounts select twitch_user
./twfarmer start --account twitch_user welovegames
./twfarmer stop
./twfarmer reload
./twfarmer quit
```

Commands that change channels, settings, or the saved account selection apply
those changes to the running program. Temporary lists and selections are not
written to disk. `settings set` accepts `--max-channels`,
`--poll-interval-seconds`, `--watch-interval-seconds`, `--claim-bonus true|false`,
`--client-id`, `--chat-presence true|false`, `--language en|ru`, and `--oauth-scopes "user_read"`.
An empty scopes string clears the list.

The plain-log menu action releases the dashboard and disables keyboard control;
farming continues. `journal` restores the interface in an interactive terminal.
One instance can run per configuration path. `*.control.json` and
`*.control.lock` are excluded from Git. The local control address uses a separate
random key, not a Twitch token.

## systemd

Start only already configured accounts and sessions:

```bash
./twfarmer --config /absolute/path/twfarmer.toml daemon
```

`daemon` never opens the interface or reads stdin. It requires an existing
configuration, saved sessions, selected accounts, and configured channels.
Missing setup produces an error in the log. It does not start device sign-in.
Saved sessions are validated and refreshed as usual; run `login` separately
if an account needs to sign in again.
It uses `enabled_accounts`, or all saved accounts when that selection is absent.
Use `daemon --account LOGIN1,LOGIN2` to override it. Channels come from saved
shared and personal lists. Language follows the saved configuration or `--lang`.
`SIGTERM` and `Ctrl+C` exit with code 0 and release the instance lock.

The user unit `packaging/twfarmer.service` uses `~/.local/bin/twfarmer` and
`~/.config/twfarmer/twfarmer.toml`. From the extracted Linux archive:

```bash
install -Dm755 twfarmer ~/.local/bin/twfarmer
mkdir -p ~/.config/twfarmer ~/.config/systemd/user
cp packaging/twfarmer.service ~/.config/systemd/user/twfarmer.service
~/.local/bin/twfarmer --config "$HOME/.config/twfarmer/twfarmer.toml" ui
```

Save accounts and channels in the interface, then close the program.
For existing setup, first move `twfarmer.toml`, the `twfarmer.toml.accounts`
directory, and any legacy `twfarmer.toml.auth.json` to `~/.config/twfarmer`.
Keep their names and permissions. For a source build, use
`target/release/twfarmer` instead of `twfarmer` in the `install` command.

```bash
systemctl --user daemon-reload
systemctl --user enable --now twfarmer.service
systemctl --user status twfarmer.service
journalctl --user -u twfarmer.service -f
systemctl --user reload twfarmer.service
systemctl --user stop twfarmer.service
```

To start at boot before signing in and continue after signing out:

```bash
loginctl enable-linger "$USER"
```

The service stays in the foreground under systemd. On failure, systemd retries
after 30 seconds. Use the same absolute `--config` path for control commands.

## Configuration and sessions

Settings are relative to the working directory unless `--config PATH` is given.
See `twfarmer.example.toml`:

- `language`: `en` or `ru`; defaults to `en`.
- `channels`: shared channels, in priority order.
- `[account_channels]`: personal lists, e.g. `twitch_user = ["welovegames"]`.
- `enabled_accounts`: saved login selection; empty means none, absent means all.
- `max_channels`: 1 or 2 per account.
- `poll_interval_seconds`: balance polling, 30-3600 seconds; defaults to 60.
- `watch_interval_seconds`: watch requests, 15-60 seconds; defaults to 20.
- `claim_bonus`: automatically claim available bonuses.
- `chat_presence`: join the configured channels' chats; defaults to `true`.
- `client_id`, `oauth_scopes`: sign-in settings, defaulting to the public TV
  client ID and `user_read`. Changing the client ID requires signing in again.

New sessions are stored in `twfarmer.toml.accounts/TWITCH_LOGIN.auth.json`.
Legacy `twfarmer.toml.auth.json` files remain supported. On Unix, new session
files use permissions 0600 and account directories 0700.
Sessions contain tokens and are excluded from Git. Tokens are not printed in
the log or sent to the video CDN. Renewing an account's sign-in rejects a
different Twitch account and preserves the previous session.

Sessions are checked at startup and every five minutes. TV tokens with
`expires_in = 0` remain valid when Twitch validation succeeds.
Network failures trigger retries with backoff up to five minutes.
Press `Esc` to cancel device sign-in.

The program downloads a low-quality video segment and sends `minute-watched`
events every 20 seconds by default. Video is not played or saved to disk.
The event URL comes from Twitch settings; `use_beacon_url = true` selects
`beacon_url`. The viewer ID is sent as a number.

Twitch decides whether to award points. On a new channel, the first points
usually arrive after a few minutes. The official API does not support points
farming; internal requests can change. Automated farming may conflict with
Twitch's rules. Predictions and spending points are not implemented.

## Development

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Tests cover account migration and isolation, interface actions, saved settings,
session totals, English/Russian help and language persistence, OAuth, HLS,
Twitch errors, and release packaging. Confirming actual points for multiple
accounts requires signing in to each account and active streams.

## Releases and license

TwFarmer is distributed under the [MIT license](LICENSE) and is not affiliated
with Twitch. See [CHANGELOG.md](CHANGELOG.md) and
[RELEASING.md](https://github.com/Rednelss/TwFarmer/blob/main/RELEASING.md).

When reporting a problem, include the program version, operating system, and
error text. Do not attach `*.auth.json` or `*.control.json` files. Remove personal
data from logs before sharing them.
