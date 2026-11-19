## 1.3.1

- A proper app icon in the Plugins store and sidebar (`icons/app-icon.svg`).

## 1.3.0

- Alert Pager: the latest follow, sub, resub, gifted subs, raid or cheer through Twitch EventSub
  over WebSocket. Flashes on a new alert, stays lit until pressed, then pages back through the
  last 20; each kind can be switched off per key.
- Setup asks for three more read permissions (`moderator:read:followers`,
  `channel:read:subscriptions`, `bits:read`). Existing sign-ins keep working; run setup again for
  follows, subs and cheers on the pager (raids need no permission).
