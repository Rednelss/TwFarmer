TwFarmer 0.2.0 adds Drops farming and updates the terminal controls.

- Watch-based Drops, daily rewards, and automatic claims. Progress and claim
  status come from Twitch and appear alongside channel points.
- Available campaigns for each account, with eligibility and connection checks.
  Enable **Find Drops channels automatically** to discover participating streams,
  even with an empty channel list.
- **Points first** is the default. Choose **Drops first** to reserve one stream
  for Drops. With two slots, the other can earn points; a stream earning both
  occupies one slot. Each account uses at most one Drops stream.
- Grouped menu and settings switches operated with arrows or Space. F2 opens
  advanced settings. Use `d` for Drops, `p` for settings, and `?` for help.
- Higher-contrast channel states and log timestamps; help can be scrolled.
- The Drops browser starts with eligible rewards. Tab shows all campaigns and
  their requirements; `r` refreshes. Enter expands selected reward details for scrolling.
- Fewer repeated logs, catalog copies, and unnecessary campaign scans. Automatic
  stream changes keep configured chats connected.

Existing configuration and saved sessions remain compatible. Replace the binary
and keep the same working directory or `--config` path. Restart the systemd user
service after updating. Automatic discovery is off unless `auto_drops` is enabled.

Twitch controls credited minutes and reward delivery. Account linking, purchases,
and manual reward choices stay on Twitch. Simultaneous viewing of different
Drops campaigns did not provide stable progress and is not scheduled.
Bot loyalty balances remain separate from Twitch channel points.

Download the archive for your system and extract it. Windows uses `twfarmer.exe`;
Linux and macOS use `twfarmer`. Archives include English and Russian documentation,
an example configuration, and the MIT license. Linux also includes a systemd user
unit. GitHub Actions Linux builds require glibc 2.35 or newer.
Verify archive checksums against `SHA256SUMS` attached to the release.
