# Speed Test

Test your internet speed from a key and keep an eye on latency. Works on Windows, macOS and Linux.
No account or key needed.

## What it does

- **Speed Test**: press to measure download, upload and latency (about 17 seconds). The key shows
  all three, or only the one you pick. It can also test again by itself every 15 minutes, every
  hour or every 6 hours. Every Speed Test key shows the latest result.
- **Latency**: a live tile with the time to reach a host (`one.one.one.one` by default, or your
  router, a game server, a website), refreshed every 5 seconds.

## How it works

The test uses Cloudflare's speed test servers (the ones behind speed.cloudflare.com): it downloads
and uploads chunks for 8 seconds each way and reports the speed over the chunks after the first.
Latency is the median of several small requests. The Latency tile measures how long opening a
connection takes (port 443 unless you give one, e.g. `192.168.1.1:80`), which needs no special
permission, unlike ping.
