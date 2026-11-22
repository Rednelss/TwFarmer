TwFarmer 0.1.1 adds chat presence while farming. Each active account joins the
chats of all channels in its shared, personal, or temporary list, including
offline channels.

- Encrypted IRC connection with keepalive, reconnection, and refreshed credentials.
- A channel that rejects membership does not disconnect the other chats.
- Chat state and errors in channel details; a `CHAT` column on wide terminals.
- `chat_presence` is enabled by default and can be changed in **Farming settings >
  Join chat** or with `settings set --chat-presence false`.

Stopping farming, checking balances, or exiting closes the chat connection.
The program sends no chat messages and does not store incoming messages.

Existing configuration and session files remain compatible. After replacing
the binary, use the same working directory or `--config` path as before.
For systemd, replace the installed binary and restart the user service.

StreamElements and other bots maintain separate loyalty balances. Awarding
points for chat presence depends on the broadcaster's settings; some rules
require activity or a live stream. The dashboard shows Twitch channel points.

Extract the archive for your system and run `twfarmer` (`twfarmer.exe` on Windows).
Archives include English and Russian documentation, example configuration,
and the MIT license. Linux also includes a systemd user unit.
The GitHub Actions Linux build requires glibc 2.35 or newer.
Archive checksums are in `SHA256SUMS`.
