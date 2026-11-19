# Clocks

Clocks and timers on your keys, on Windows and macOS.

## What it does

- Clock: the time in this computer's time zone or any other (daylight saving included),
  24- or 12-hour, optionally with the date and a name such as "Tokyo".
- World Clock wheel: several time zones at once; "+1" / "-1" marks another day.
- Countdown Timer: press to start or pause; it shows "Time's up" at zero, and the next press resets
  it. Lengths like `5m`, `90s`, `1h30m` or `10:00`. Optionally it writes the time left to a text
  file every second, for an OBS Text source ("Read from file"): `countdown.txt` in the plugin's data
  folder unless you pick a path (a relative path is inside the data folder), as mm:ss / h:mm:ss,
  mm:ss with minutes past 59, or hh:mm:ss, with optional text shown at zero.
- Pomodoro: work, short break and long break (every N work sessions), lengths of your choice. Press
  to start or pause; when a phase ends the key shows "Break time" / "Focus time" and the next press
  starts it (or turn on "Start the next phase on its own"). The key shows the work sessions done
  (✓3). A Pomodoro Skip / Reset key skips to the next phase or resets the Pomodoro with the same
  name. Timers and the count start over when KnobDeck restarts.
- Stopwatch: press to start, press to stop, press again to reset.

Time zones come from macOS's own database, and from one bundled in the plugin on Windows (which has
no IANA time-zone database).
