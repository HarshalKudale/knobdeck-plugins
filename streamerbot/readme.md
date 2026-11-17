# Streamer.bot

Run Streamer.bot actions from KnobDeck. Streamer.bot then reaches Twitch, YouTube, Kick, OBS and
everything else you've set up in it. Windows only, like Streamer.bot.

## What it does

- **Run Action**: run any action, with optional arguments (`user=somebody; amount=5`; the action
  reads them as `%user%`, `%amount%`). Arguments accept KnobDeck placeholders such as
  `{clipboard}`.
- **Run Code Trigger**: fire a custom trigger a C# action registered.
- **Actions Wheel**: the enabled actions of a group (or all), up to 16, as a wheel.
- **Global Variable**: show a global variable's value on a key, refreshed every 2 seconds.

## Setup

In Streamer.bot, open **Servers/Clients ▸ WebSocket Server** and turn it on (it listens on
`127.0.0.1:8080` by default). If you changed the address or port, or turned on authentication,
enter them in the plugin's setup; the password is kept in this computer's credential store.

## How it works

It talks to Streamer.bot's WebSocket server: `GetActions`, `DoAction`, `ExecuteCodeTrigger`,
`GetCodeTriggers` and `GetGlobal`. Streamer.bot has no request for changing a global variable, so
that is left to an action.
