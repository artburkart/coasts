# Host Mounts

The `[host_mounts.*]` sections declare extra host directories to bind-mount into the Coast container at runtime. This is the escape hatch for sibling repos, shared local assets, and other host paths that live outside the normal project root mount.

These mounts are available to:

- `coast exec`
- bare `[services.*]`
- `[agent_shell]`
- inner compose services, by referencing the declared target path

They are **not** available during `[coast.setup]` image build steps. `[coast.setup]` runs while building the custom DinD base image, before any instance-specific runtime bind mounts exist.

## Syntax

```toml
[host_mounts.omai_packs]
source = "../omai-packs"
target = "/host-mounts/omai-packs"
```

Each mount is a named TOML section under `[host_mounts]`.

### `source`

Path on the host to mount.

- Relative paths are resolved against the Coast project root.
- `~/...` expands to your home directory.
- Absolute paths are allowed.

```toml
source = "../omai-packs"
source = "~/.codex/worktrees"
source = "/opt/shared-rules"
```

### `target`

Absolute path inside the Coast container where the host directory will appear.

```toml
target = "/host-mounts/omai-packs"
```

Rules:

- must be absolute
- must not contain `..`
- must not end with `/`
- must not collide with Coast-reserved paths such as `/workspace`, `/host-project`, `/coast-artifact`, `/coast-override`, `/image-cache`, `/run/secrets`, `/coast-volumes`, or `/host-external-wt`

### `read_only`

Whether the mount is read-only inside the Coast container. Defaults to `true`.

```toml
[host_mounts.cache]
source = "../shared-cache"
target = "/host-mounts/shared-cache"
read_only = false
```

## Example: Sibling Repo for Compose

Mount a sibling repo into the Coast container, then let an inner compose service bind from that stable target:

```toml
[coast]
name = "omai-demo"
compose = "./docker-compose.coasts.yml"

[host_mounts.omai_packs]
source = "../omai-packs"
target = "/host-mounts/omai-packs"
```

```yaml
services:
  backend:
    volumes:
      - /host-mounts/omai-packs:/opt/omai-packs:ro
```

This avoids changing `root` just to make a sibling directory visible.

## Lifecycle

Host mounts are created when the Coast instance is created. If you add, remove, or change a `[host_mounts.*]` entry, recreate the instance so Docker applies the new bind mount set.
