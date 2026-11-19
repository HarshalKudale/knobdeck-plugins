# Changelog

## 1.2.0

- Keys follow entities live over Home Assistant's WebSocket API (`/api/websocket`, the same
  long-lived token): a change made in Home Assistant, by an automation or a wall switch shows at
  once instead of at the next poll. `wss://` (Nabu Casa, a reverse proxy) works too.
- When the WebSocket connection drops the plugin polls over REST as before, and reconnects with
  back-off (1 s up to 60 s).
