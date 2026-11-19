# Network

Network status and switches on KnobDeck keys. Works on Windows, macOS and Linux.

## What it does

- **Public IP**: your public IPv4 or IPv6 address (ipify), every 5 minutes; press to check now.
- **Wake on LAN**: sends the magic packet for a MAC address to the broadcast address (or your
  network's, like `192.168.1.255`), port 9 or 7.
- **Tailscale**: on/off with your Tailscale IP. **Tailscale Exit Node**: use one exit node, or
  stop using it. **Tailscale Exit Nodes**: a wheel of your tailnet's exit nodes plus "No exit node".
- **Wi-Fi Network** (Linux, Windows): join or leave a saved network; the key shows whether it is
  connected.
- **VPN** (Linux, Windows, macOS): connect or disconnect a VPN set up in the system.
- **Pi-hole Blocking**: shows whether blocking is on and today's blocked share; pressing pauses
  blocking for the minutes you choose (0: until pressed again) or turns it back on.
- **Uptime Kuma Status**: "All 5 up" or which monitors are down, from a public status page;
  pressing opens the page.

## How it works

- **Tailscale** through its LocalAPI, the interface the `tailscale` CLI uses. On Linux, changing
  settings needs root or the operator: run `sudo tailscale set --operator=$USER` once.
- **Wi-Fi and VPN**: Linux NetworkManager over D-Bus (saved Wi-Fi connections; VPN and WireGuard
  connections). Windows: saved Wi-Fi profiles through the Native Wi-Fi API; VPN entries from
  Settings ▸ Network ▸ VPN through RAS, with their saved sign-in info. macOS: VPN services from
  System Settings ▸ VPN through `scutil --nc`. Switching Wi-Fi networks isn't offered on macOS:
  macOS has no way for an app to join a saved network without asking for its password again.
- **Pi-hole** v6 REST API. Setup takes its address and password (the web password or an app
  password; kept in this computer's credential store). Use `http://`: self-signed HTTPS
  certificates are not accepted.
- **Uptime Kuma**: the status page's public API; no login.
