# Mixed Service Types

Most projects fit neatly into one model: a `docker-compose.yml` that orchestrates everything, or `[services]` that run plain processes. Some projects need both at the same time. Coast supports defining `compose` and `[services]` in the same Coastfile so that Docker Compose manages your heavyweight, stateful services while bare processes handle lightweight tooling that belongs on the DinD host.

## When You Need Mixed Services

The most common scenario is a large monorepo where the core application stack (web servers, databases, background workers) is already containerized via Docker Compose, but certain development tools run better as host-level processes:

- **Vite / Webpack dev servers** that need to be reachable from inside compose containers via `host.docker.internal`. Running them as bare services on the DinD host avoids networking hacks.
- **File watchers or code generators** that need fast filesystem access to the bind-mounted `/workspace` without going through an inner container's overlay filesystem.
- **Language servers or build tools** that are simpler to run as a process than to bake into a container image.

If your compose services need to reach a host-level process by hostname, mixed services are the right answer.

## Configuration

Set `compose` for your Docker Compose stack and add `[services.*]` sections for bare processes in the same Coastfile:

```toml
[coast]
name = "my-app"
compose = "./docker-compose.yml"

[coast.setup]
packages = ["nodejs", "npm"]

# Bare services run directly on the DinD host
[services.vite-web]
install = "cd /workspace && npm install"
command = "cd /workspace && npm run dev"
port = 3040
restart = "on-failure"

# Compose services are defined in docker-compose.yml as usual
# (web, postgres, redis, sidekiq, etc.)

[ports]
web = 3000
vite-web = 3040
postgres = 5432

[assign]
default = "none"

[assign.services]
web = "restart"
sidekiq = "restart"
vite-web = "restart"
```

The `[coast.setup]` section installs packages needed by bare services. Compose services get their runtimes from their Dockerfiles as usual.

## How It Works

On `coast run`, Coast starts both service types sequentially:

1. Compose services start first via `docker compose up -d` inside the DinD container and Coast waits for health checks to pass
2. Bare services start next via the supervisor scripts in `/coast-supervisor/`

Both types run concurrently once started. Compose services run as inner containers managed by the DinD daemon. Bare services run as plain processes on the DinD host OS.

```text
┌─── Coast: dev-1 ──────────────────────────────────────────┐
│                                                            │
│   Inner Docker daemon (compose services)                   │
│   ├── web           (Rails, :3000)                         │
│   ├── postgres      (database, :5432)                      │
│   └── redis         (cache, :6379)                         │
│                                                            │
│   /coast-supervisor/ (bare services)                       │
│   ├── vite-web.sh   (node process, :3040)                  │
│   └── vite-admin.sh (node process, :3041)                  │
│                                                            │
│   /workspace ← bind mount of project root                  │
└────────────────────────────────────────────────────────────┘
```

## Networking Between Service Types

Compose services run inside the inner Docker daemon's network. Bare services run on the DinD host. To let compose containers reach a bare service:

- Bare services should bind to `0.0.0.0` so they are reachable from inside the Docker network
- Compose containers can reach bare services via `host.docker.internal` (automatically available in DinD containers)
- Set environment variables in your compose file to point at the bare service: `VITE_HOST=host.docker.internal:3040`

Bare services can reach compose containers via `localhost:<published-port>` since compose ports are published on the DinD host.

## Commands

All Coast commands work with both service types transparently:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | Shows with `kind: compose` | Shows with `kind: bare` |
| `coast logs` | Fetches via `docker compose logs` | Tails from `/var/log/coast-services/` |
| `coast logs --service <name>` | Routes to compose if compose service | Routes to bare if bare service |
| `coast restart-services` | Runs compose down + up | Runs stop-all + start-all |
| `coast assign` | Handles compose rebuild/restart | Re-runs install commands and restarts |
| `coast stop` / `coast start` | Stops/starts compose | Stops/starts bare services |

When requesting logs for a specific service, Coast determines the service type automatically and routes to the right log source.

## Branch Switching

On `coast assign`, both service types are handled:

1. Compose services are torn down or force-recreated depending on the assign strategy (`restart`, `rebuild`, `hot`)
2. Bare services are stopped, install commands re-run, and services restart

The `[assign.services]` section accepts both compose service names and bare service names. Use the same strategies for both:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## Shared Services

[Shared services](SHARED_SERVICES.md) work independently of both compose and bare services. They run on the host Docker daemon and are accessible from inside the Coast regardless of which service types are configured.

## Migrating from Bare-Only to Mixed

If you started with a bare-services-only Coastfile and want to containerize some services while keeping others as bare processes:

1. Write Dockerfiles and a `docker-compose.yml` for the services you want to containerize
2. Add `compose = "./docker-compose.yml"` to the `[coast]` section
3. Remove the `[services.*]` sections for services that are now in compose
4. Keep `[services.*]` for processes that should remain bare
5. Rebuild with `coast build`
