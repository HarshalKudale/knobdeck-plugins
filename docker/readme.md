# Docker

Start, stop and restart containers and Compose projects from KnobDeck and see what is running.
Works on Windows, macOS and Linux with Docker Desktop, Docker Engine (also rootless), Colima,
OrbStack and Podman.

## What it does

- **Start / Stop Container**: one key that shows whether the container runs and switches it.
- **Start Container**, **Stop Container**, **Restart Container**.
- **Container Status**: name and state (`Up 3 hours`, `healthy`, `Stopped`) on a key.
- **Start / Stop Compose Project**: every container of a Compose project at once; the key shows
  how many are up. A project with any container up is stopped; otherwise all are started.
  (Containers that `docker compose up` hasn't created yet can't be started from here.)
- **Running Containers**: `3 / 7 running` on a key.
- **Containers Wheel**: up to 16 containers; a slice starts or stops one.

## Setup

None. The plugin finds the engine by itself: `DOCKER_HOST` when set, then the usual sockets
(`/var/run/docker.sock`, rootless Docker's and Podman's sockets in `$XDG_RUNTIME_DIR`, Docker
Desktop's, OrbStack's and Colima's under your home folder) or, on Windows, the `docker_engine` and
`podman-machine-default` named pipes. On Linux your user needs access to the socket (the `docker`
group, rootless Docker, or `systemctl --user enable --now podman.socket`).

## How it works

It talks to the Docker Engine API directly over that socket (Podman serves the same API); it never
runs the `docker` command.
