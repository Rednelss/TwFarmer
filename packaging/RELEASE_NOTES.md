TwFarmer 0.3.0 adds automatic GitHub updates and replaces device-code login with native token sign-in. Linked Drops campaigns can start before their first watch minute.

- Automatic updates are enabled by default. TwFarmer checks at startup and every
  24 hours, downloads the archive for its platform, verifies `SHA256SUMS`, and
  restarts with active accounts and channels. Linux retains the service PID.
- Use `update --check` to check without installing, or `update` to install.
  Both actions are available in the menu. Disable automatic updates with
  `settings set --auto-update false` or the farming settings form.
- `login` accepts a native token or its Twitch redirect URL with hidden input.
  Twitch determines the account name. `F2` copies the complete sign-in URL.
  The program runs without a browser on the host, including a VDS.
- Drops linkage is checked per campaign and account. Linked rewards no longer
  require a browser watch to initialize progress; unlinked games remain excluded.
- Existing channel lists and settings remain compatible. Obsolete Client ID and
  OAuth scope settings are ignored. Daily reward windows continue to use Twitch's
  account-specific reset times.

When upgrading from 0.2.x, install the 0.3.0 binary manually once. Old TV sessions
that cannot read campaign links require a new `login` for each Twitch account;
keep the same directory or `--config` path to retain channel lists.

Release builds: Linux x86_64 (glibc 2.35+), Windows x86_64, macOS Apple Silicon.
Updates require write access to the executable directory. A running instance
updated through a separate `update` command needs a restart to use the new version.
Verify downloaded archives against the release's `SHA256SUMS`.
