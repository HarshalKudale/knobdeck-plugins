# Coding Agents

See what your Claude Code and Codex sessions are doing, and approve or deny Claude Code's
permission requests from a key. Works on Windows, macOS and Linux.

## What it does

- **Agent Status**: the project and state of a session: *Working*, *Needs approval*, *Needs
  input*, *Your turn* (it finished) or *Idle*, with how long it has been that way. A session that
  needs you always shows first; pressing the key steps through the others (`2/3`).
- **Needs Me**: how many sessions need you, or how many are working. Pressing it marks finished
  sessions as seen.
- **Approve** / **Deny**: answer the oldest waiting Claude Code permission request. The key shows
  what it is about (`Bash: npm test`, `Write: main.rs`).
- **Approvals Wheel**: Allow and Deny slices for each waiting request.
- **Sessions Wheel**: every session and its state.

## Setup

Setup connects the agents you tick, through their own hook interfaces:

- **Claude Code**: adds KnobDeck's hooks to `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`)
  for SessionStart, UserPromptSubmit, PostToolUse (async), Notification, PermissionRequest, Stop
  and SessionEnd. Your own hooks and settings stay as they are.
- **Codex**: sets `notify` in `~/.codex/config.toml` (or `$CODEX_HOME`). Codex runs only one
  notify program; if you already have one, setup says so and leaves it alone.

Each file is backed up once (`settings.json.knobdeck-backup`, `config.toml.knobdeck-backup`) before
the first change. Running setup again with an agent unticked removes only KnobDeck's entries.
Restart running sessions after setup so they read the new settings.

**Approving from a key**: while an Approve, Deny or Approvals Wheel key is placed and KnobDeck is
running, a Claude Code permission request waits for your press (30 seconds by default; 15 s to
2 min, or off, in setup). When the time is up, or KnobDeck isn't running, Claude Code shows its
usual prompt in the terminal. Without such a key nothing waits.

## How it works

Hooks run this plugin's own program (`bin/knobdeck-agent-hook` in the plugin's data folder, kept
up to date by the plugin), which records each session as a small file there; it adds about a
millisecond to each event and never blocks the agent except for the approval wait above. Codex
reports only when a turn ends, so its *Working* state comes from its session log
(`~/.codex/sessions`) being written. Nothing is sent anywhere.

The plugin doesn't bring a session's terminal to the front or start new sessions: KnobDeck
doesn't focus other apps, and which terminal you use differs per machine.
