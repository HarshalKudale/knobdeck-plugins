# Toolbox

Small everyday tools on KnobDeck keys and dials. Windows, macOS and Linux. Everything runs on
this computer: no account, no network.

## What it does

- **Counter**: a press adds, subtracts or resets (choose the step and the reset value). The count
  is kept across restarts. Keys and dials with the same counter name share one count, so one key
  can add and another reset.
- **Counter Dial**: turn the dial to count up and down (choose how much one tick counts).
- **Roll Dice**: d4, d6, d8, d10, d12, d20 or d100, one to six at a time ("3 + 5 = 8").
- **Flip a Coin**: heads or tails.
- **Random Pick**: picks one line from your own list (never the same one twice in a row, unless
  you turn that off).
- **Tally to File**: each press adds a row with the date and time to a CSV file (`time,entry`);
  the key shows how many rows of its entry the file holds. Several keys can write different
  entries to the same file. Without a file chosen it writes `KnobDeck Tally.csv` in your home
  folder.

## Setup

None.

## How it works

Counters are saved in the plugin's data folder (`counters.json`). Random numbers come from the
OS-seeded hasher in Rust's standard library, drawn without modulo bias. Nothing is sent anywhere
and no keyboard shortcuts are used.
