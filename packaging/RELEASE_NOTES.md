TwFarmer 0.2.1 fixes repeated viewing after a daily Drops reward has been granted.

- Daily rewards release their video slot until the next viewing window. Automatic
  discovery can then select another eligible campaign, including after a restart.
- Reset times come from Twitch's `nextEligibleAt` or `currentWindow.expiresAt`,
  separately for each account and reward.
- Watch requirements come from Twitch, including daily rewards requiring 90 minutes.
- Completed single rewards and fully earned repeats are excluded from automatic viewing.
- When Twitch omits the reset time, the program shows that it is waiting for a
  reset and checks for new progress or eligibility.

Existing configuration and saved sessions remain compatible. Replace the binary
and restart TwFarmer or the systemd user service, keeping the same working
directory or `--config` path.

Twitch controls credited progress and reward delivery. Verify the downloaded
archive against `SHA256SUMS` attached to the release.
