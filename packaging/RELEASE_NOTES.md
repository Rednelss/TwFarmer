First public release of TwFarmer, a Rust terminal client for Twitch channel
points farming with multiple accounts.

- Twitch device-code sign-in; passwords are entered on Twitch.
- Shared and personal channel lists for each account.
- Updating interface with balances, bonuses, states, and logs.
- Menus and commands, up to two concurrent channels per account.
- `daemon` for saved sessions and a systemd user unit.
- English and Russian interface, help, and messages. Choose **Language / Язык**
  in the menu, use `twfarmer language en|ru`, or override a run with `--lang`.

Archives contain the program, English and Russian README and changelog files,
example configuration, and the MIT license. Linux also includes the systemd unit.
Session files are excluded.

Extract the archive for your system and run `twfarmer` (`twfarmer.exe` on Windows).
Add an account, set channels, and start farming through the menu.
The GitHub Actions Linux build requires glibc 2.35 or newer.
Archive checksums are in `SHA256SUMS`.

Twitch decides whether to award points. The program uses internal requests
that can change. See the README for instructions and limitations.
