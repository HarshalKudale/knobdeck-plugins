# Neovim

Drive a running Neovim from KnobDeck. Works on Windows, macOS and Linux.

## What it does

- **Run Command**: one or more Ex commands, one per line (`:write`, `:Telescope find_files`…).
  Placeholders such as `{selection}`, `{clipboard}` and `{date}` work in the text.
- **Run Lua**: a Lua snippet; optionally shows what it returns.
- **Save All**: `:wall`.
- **Switch Buffer**: turn the dial to go to the next or previous buffer, or drag the slider to
  pick one; the key shows the buffer's name and its place in the list.
- **Toggle Option**: switches an on/off option (wrap, relativenumber, number, spell, list… or
  any other) and shows whether it is on.
- **Mode and File**: the mode (NORMAL, INSERT, VISUAL…) and the current file, with ● when it has
  unsaved changes; pressing it can save the file.

Each action works with **Newest Neovim** (the most recently started one that answers) or with a
Neovim you pick from the list, or an address you type.

## Setup

None for Neovim 0.9 or newer: every Neovim starts a server and the plugin finds it
(`$XDG_RUNTIME_DIR/nvim.<pid>.0` on Linux, a folder under `$TMPDIR` on macOS,
`\\.\pipe\nvim.<pid>.0` on Windows). A Neovim started with `--listen <address>` (a socket path,
a pipe name or `host:port`) is reached by typing that address in the action, or by putting
`NVIM_ADDRESS=<address>` in the plugin's `.env`.

## How it works

Neovim's own msgpack-RPC API (`nvim_command`, `nvim_exec_lua`, `nvim_get_mode`) on its server
socket or pipe; it never types keys into Neovim. When Neovim waits at a prompt, actions report
that instead of waiting.
