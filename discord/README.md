# Discord Plugin for KnobDeck

Discord integration plugin — control mute, deafen, push-to-talk, channels, volume, soundboard, and more directly from your KnobDeck hub.

## Features

| Action | Type | Description |
|--------|------|-------------|
| **Mute** | Toggle | Toggle microphone mute |
| **Deafen** | Toggle | Toggle headphone deafen |
| **Push to Talk** | Hold | Hold to transmit |
| **Push to Mute** | Hold | Hold to mute |
| **Voice Mode Toggle** | Toggle | Switch between Voice Activity ↔ Push-to-Talk mode |
| **Voice Channel** | Toggle | Join / leave a voice channel |
| **Text Channel** | Button | Navigate to a text channel |
| **Server Jump** | Button | Jump to a server without manually finding channel IDs |
| **Open Direct Message** | Button | Open a DM from a friend picker |
| **Video Toggle** | Toggle | Turn camera on/off in calls |
| **Stream Toggle** | Toggle | Start/stop screen sharing |
| **Volume Control** | Button | Adjust input/output device volume |
| **User Volume Control** | Button | Adjust per-user local volume |
| **Set Audio Device** | Button | Switch input/output audio device |
| **Soundboard** | Button | Play a Discord Soundboard sound |
| **Voice Channel Wheel** | Dynamic Wheel | Pick a voice channel from a server live |
| **Call Member Mute Wheel** | Dynamic Wheel | Mute / unmute current call members without user IDs |
| **Activity Preset** | Button | Set or clear Discord activity text |
| **Soundboard Wheel** | Dynamic Wheel | Trigger sounds from a live sound picker |
| **Notifications** | Toggle | Show & manage unread notifications |
| **Server Stats** | Button | Display online user / message counts |

---

Most Discord actions now prefer **server, channel, friend, device, sound, and call-member pickers** over raw guild/channel/user IDs.

---

## Setup

1. Open the **Discord desktop app** and sign in.
2. In KnobDeck, open **Plugins → Discord** and click **Setup**.
3. Optionally enter your own Discord app's **Client ID** (see below), then click **Authorize Discord**.
4. Discord shows an authorization popup inside the client. Click **Authorize**.

That's it. The sign-in is kept in your OS credential store (macOS Keychain, Windows Credential
Manager, or the Secret Service on Linux: KWallet / GNOME Keyring) and renewed on its own, so you only
run setup again if you revoke access in Discord (**User Settings → Authorized Apps**). On a Linux
session without a Secret Service it stays in the plugin data folder, readable by your user only.

### Using your own Discord app (Client ID)

Discord lets an app use RPC only after Discord approves it. Until KnobDeck's Discord app is
approved, only a short list of test accounts can authorize it, and setup fails for everyone else
with `invalid_scope`. This is Discord's restriction. To use the plugin now, create your own app:

1. Open <https://discord.com/developers/applications>, sign in with the account you use in the
   Discord app, and click **New Application**.
2. Open **OAuth2** and copy the **Client ID**.
3. On the same page, add `http://localhost` under **Redirects**, turn on **Public Client**, and save.
4. Paste the Client ID into the setup window.

You don't need a client secret: the app is a public client, so setup signs in with PKCE and
talks to Discord directly. The Client ID is saved with the sign-in in the OS credential store,
and token renewals use the same app. Leave the field blank to use KnobDeck's app.

---

## Permissions

Setup asks Discord for these scopes:

| Scope | Purpose |
|-------|---------|
| `identify` | Identify the current user for RPC auth |
| `rpc` | Base RPC connection to Discord |
| `rpc.voice.read` | Read voice settings and channel state |
| `rpc.voice.write` | Control mute, deafen, push-to-talk and volume |
| `guilds` | List servers for channel pickers |
| `messages.read` | Server stats and notifications |

---

## Troubleshooting

- **"Discord is not connected"**: Discord must be running. The plugin reconnects on its own when it starts.
- **Setup fails**: make sure you are signed in to the Discord desktop app (not only the browser) and try again.
- **Setup fails with `invalid_scope`**: Discord only lets an app's owner and its listed App Testers authorize RPC until Discord approves the app. Use your own app's Client ID (above). With your own app, sign in to Discord with the account that created it.
- **Setup fails with `invalid_client`** (your own app): turn on **Public Client** in the app's OAuth2 settings.
- **Actions stop working after a while**: run **Setup** again; this happens when access was revoked in Discord.

The plugin talks to the Discord desktop client over its local RPC pipe, the same channel Discord's
own overlays use. Nothing is scraped and no browser is involved.
