# Next.js Application

This recipe is for a Next.js application backed by Postgres and Redis, with optional background workers or companion services. The stack runs Next.js as a [bare service](../concepts_and_terminology/BARE_SERVICES.md) with Turbopack for fast HMR, while Postgres and Redis run as [shared services](../concepts_and_terminology/SHARED_SERVICES.md) on the host so every Coast instance shares the same data.

This pattern works well when:

- Your project uses Next.js with Turbopack in development
- You have a database and cache layer (Postgres, Redis) backing the application
- You want multiple Coast instances running in parallel without per-instance database setup
- You use auth libraries like NextAuth that embed callback URLs in responses

## The Complete Coastfile

```toml
[coast]
name = "my-nextjs-app"
primary_port = "web"
private_paths = ["packages/web/.next"]
worktree_dir = [".worktrees", ".claude/worktrees"]

[coast.setup]
packages = ["nodejs", "npm", "make", "git", "bash"]
run = [
    "npm install -g corepack",
    "corepack enable",
]

# --- Bare services: Next.js and background worker ---

[services.web]
install = [
    "cd /workspace && (test -f node_modules/.yarn-state.yml || make yarn)",
    "cd /workspace && test -f config.json || echo {} > config.json",
    "cd /workspace && DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5432/postgres yarn prisma migrate dev",
]
command = "cd /workspace && AUTH_URL=http://localhost:${WEB_DYNAMIC_PORT:-3000} DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5432/postgres REDIS_URL=redis://host.docker.internal:6379 yarn dev"
port = 3000
restart = "on-failure"
cache = ["node_modules"]

[services.worker]
install = "cd /workspace && (test -f node_modules/.yarn-state.yml || make yarn)"
command = "cd /workspace && DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5432/postgres REDIS_URL=redis://host.docker.internal:6379 yarn dev:worker"
restart = "on-failure"
cache = ["node_modules"]

# --- Shared services: Postgres and Redis on the host ---

[shared_services.postgres]
image = "postgres:16-alpine"
ports = [5432]
volumes = ["myapp_postgres_data:/var/lib/postgresql/data"]
env = { POSTGRES_DB = "postgres", POSTGRES_USER = "postgres", POSTGRES_PASSWORD = "postgres" }

[shared_services.redis]
image = "redis:7-alpine"
ports = [6379]
volumes = ["myapp_redis_data:/data"]

# --- Secrets: connection strings for bare services ---

[secrets.database_url]
extractor = "command"
run = "echo postgresql://postgres:postgres@host.docker.internal:5432/postgres"
inject = "env:DATABASE_URL"

[secrets.redis_url]
extractor = "command"
run = "echo redis://host.docker.internal:6379"
inject = "env:REDIS_URL"

# --- Ports ---

[ports]
web = 3000
postgres = 5432
redis = 6379

# --- Assign: branch-switch behavior ---

[assign]
default = "none"
exclude_paths = ["docs", ".github", "scripts"]

[assign.services]
web = "hot"
worker = "hot"

[assign.rebuild_triggers]
web = ["package.json", "yarn.lock"]
worker = ["package.json", "yarn.lock"]
```

## Project and Setup

```toml
[coast]
name = "my-nextjs-app"
primary_port = "web"
private_paths = ["packages/web/.next"]
worktree_dir = [".worktrees", ".claude/worktrees"]
```

**`private_paths`** is critical for Next.js. Turbopack creates a lock file at `.next/dev/lock` on startup. Without `private_paths`, a second Coast instance on the same branch sees the lock and refuses to start. With it, each instance gets its own isolated `.next` directory via a per-instance overlay mount. See [Private Paths](../concepts_and_terminology/PRIVATE_PATHS.md).

**`worktree_dir`** lists directories where git worktrees live. If you use multiple coding agents (Claude Code, Cursor, Codex), each may create worktrees in different locations. Listing them all lets Coast discover and assign worktrees regardless of which tool created them.

```toml
[coast.setup]
packages = ["nodejs", "npm", "make", "git", "bash"]
run = [
    "npm install -g corepack",
    "corepack enable",
]
```

The setup section installs system packages and tools needed by bare services. `corepack enable` activates yarn or pnpm based on the project's `packageManager` field. These run at build time inside the Coast image, not at instance startup.

## Bare Services

```toml
[services.web]
install = [
    "cd /workspace && (test -f node_modules/.yarn-state.yml || make yarn)",
    "cd /workspace && test -f config.json || echo {} > config.json",
    "cd /workspace && DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5432/postgres yarn prisma migrate dev",
]
command = "cd /workspace && AUTH_URL=http://localhost:${WEB_DYNAMIC_PORT:-3000} DATABASE_URL=postgresql://postgres:postgres@host.docker.internal:5432/postgres REDIS_URL=redis://host.docker.internal:6379 yarn dev"
port = 3000
restart = "on-failure"
cache = ["node_modules"]
```

**Conditional installs:** The `test -f node_modules/.yarn-state.yml || make yarn` pattern skips dependency installation if `node_modules` already exists. This makes branch switches fast when dependencies have not changed. See [Bare Service Optimization](../concepts_and_terminology/BARE_SERVICE_OPTIMIZATION.md).

**`cache`:** Preserves `node_modules` across worktree switches so `yarn install` runs incrementally instead of from scratch.

**`AUTH_URL` with dynamic port:** Next.js applications using NextAuth (or similar auth libraries) embed callback URLs in responses. Inside the Coast, Next.js listens on port 3000, but the host-side port is dynamic. Coast injects `WEB_DYNAMIC_PORT` into the container environment automatically (derived from the `web` key in `[ports]`). The `:-3000` fallback means the same command works outside of Coast. See [Dynamic Port Environment Variables](../concepts_and_terminology/DYNAMIC_PORT_ENVIRONMENT_VARIABLES.md).

**`host.docker.internal`:** Bare services cannot reach shared services via `localhost` because shared services run on the host Docker daemon. `host.docker.internal` resolves to the host from inside the Coast container.

## Shared Services

```toml
[shared_services.postgres]
image = "postgres:16-alpine"
ports = [5432]
volumes = ["myapp_postgres_data:/var/lib/postgresql/data"]
env = { POSTGRES_DB = "postgres", POSTGRES_USER = "postgres", POSTGRES_PASSWORD = "postgres" }

[shared_services.redis]
image = "redis:7-alpine"
ports = [6379]
volumes = ["myapp_redis_data:/data"]
```

Postgres and Redis run on the host Docker daemon as [shared services](../concepts_and_terminology/SHARED_SERVICES.md). Every Coast instance connects to the same databases, so users, sessions, and data are shared across instances. This avoids the problem of needing to sign up separately in each instance.

If your project already has a `docker-compose.yml` with Postgres and Redis, you can use `compose` instead and set the volume strategy to `shared`. Shared services are simpler for bare-service Coastfiles because there is no compose file to manage.

## Secrets

```toml
[secrets.database_url]
extractor = "command"
run = "echo postgresql://postgres:postgres@host.docker.internal:5432/postgres"
inject = "env:DATABASE_URL"

[secrets.redis_url]
extractor = "command"
run = "echo redis://host.docker.internal:6379"
inject = "env:REDIS_URL"
```

These inject `DATABASE_URL` and `REDIS_URL` into the Coast container environment at build time. The connection strings point at the shared services via `host.docker.internal`.

The `command` extractor runs a shell command and captures stdout. Here it just echoes a static string, but you could use it to read from a vault, run a CLI tool, or compute a value dynamically.

Note that bare service `command` fields also set these variables inline. The inline values take precedence, but the injected secrets serve as defaults for `install` steps and `coast exec` sessions.

## Assign Strategies

```toml
[assign]
default = "none"
exclude_paths = ["docs", ".github", "scripts"]

[assign.services]
web = "hot"
worker = "hot"

[assign.rebuild_triggers]
web = ["package.json", "yarn.lock"]
worker = ["package.json", "yarn.lock"]
```

**`default = "none"`** leaves shared services and infrastructure untouched on branch switch. Only services that depend on code get an assign strategy.

**`hot` for Next.js and workers:** Next.js with Turbopack has built-in hot module replacement. When Coast remounts `/workspace` to the new worktree, Turbopack detects the file changes and recompiles automatically. No process restart needed. Background workers using `tsc --watch` or `nodemon` also pick up changes through their file watchers.

**`rebuild_triggers`:** If `package.json` or `yarn.lock` changed between branches, the service's `install` commands re-run before the service restarts. This ensures dependencies are up to date after a branch switch that added or removed packages.

**`exclude_paths`:** Speeds up the first-time worktree bootstrap by skipping directories that services do not need. Documentation, CI configs, and scripts are safe to exclude.

## Adapting This Recipe

**No background worker:** Remove the `[services.worker]` section and its assign entry. The rest of the Coastfile works unchanged.

**Monorepo with multiple Next.js apps:** Add a `private_paths` entry for each app's `.next` directory. Each bare service gets its own `[services.*]` section with the appropriate `command` and `port`.

**pnpm instead of yarn:** Replace `make yarn` with your pnpm install command. Adjust the `cache` field if pnpm stores dependencies in a different location (e.g. `.pnpm-store`).

**No shared services:** If you prefer per-instance databases, remove the `[shared_services]` and `[secrets]` sections. Add Postgres and Redis to a `docker-compose.yml`, set `compose` in the `[coast]` section, and use [volume strategies](../coastfiles/VOLUMES.md) to control isolation. Use `strategy = "isolated"` for per-instance data or `strategy = "shared"` for shared data.

**Additional auth providers:** If your auth library uses environment variables other than `AUTH_URL` for callback URLs, apply the same `${WEB_DYNAMIC_PORT:-3000}` pattern to those variables in the service command.
