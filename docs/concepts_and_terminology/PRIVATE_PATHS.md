# Private Paths

When multiple Coast instances share the same project root, they share the same files — and the same inodes. This is normally the point: file changes on the host appear inside the Coast instantly because both sides see the same filesystem. But some tools write per-process state into the workspace that assumes exclusive access, and that assumption breaks when two instances share the same mount.

## The Problem

Consider Next.js 16, which acquires an exclusive lock on `.next/dev/lock` via `flock(fd, LOCK_EX)` when the dev server starts. `flock` is an inode-level kernel mechanism — it does not care about mount namespaces, container boundaries, or bind mount paths. If two processes in two different Coast containers both point at the same `.next/dev/lock` inode (because they share the same host bind mount), the second process sees the first's lock and refuses to start:

```text
⨯ Another next dev server is already running.

- Local: http://localhost:3000
- PID: 1361
- Dir: /workspace/frontend
```

The same category of conflict applies to:

- `flock` / `fcntl` advisory locks (Next.js, Turbopack, Cargo, Gradle)
- PID files (many daemons write a PID file and check it on startup)
- Build caches that assume single-writer access (Webpack, Vite, esbuild)

Mount namespace isolation (`unshare`) does not help here. Mount namespaces control which mount points a process can see, but `flock` operates on the inode itself. Two processes seeing the same inode through different mount paths will still conflict.

## The Solution

The `private_paths` Coastfile field declares workspace-relative directories that should be per-instance. Each Coast instance gets its own isolated bind mount for these paths, backed by a per-instance directory on the container's own filesystem.

```toml
[coast]
name = "my-app"
private_paths = ["frontend/.next"]
```

After Coast mounts `/workspace` with shared propagation, it applies an additional bind mount for each private path:

```text
mkdir -p /coast-private/frontend/.next /workspace/frontend/.next
mount --bind /coast-private/frontend/.next /workspace/frontend/.next
```

`/coast-private/` lives on the DinD container's writable layer — not on the shared host bind mount — so each instance naturally gets separate inodes. The lock file in `dev-1` lives at a different inode than the lock file in `dev-2`, and the conflict disappears.

## How It Works

Private path mounts are applied at every point in the Coast lifecycle where `/workspace` is mounted or remounted:

1. **`coast run`** — after the initial `mount --bind /host-project /workspace && mount --make-rshared /workspace`, private paths are mounted.
2. **`coast start`** — after re-applying the workspace bind mount on container restart.
3. **`coast assign`** — after unmounting and rebinding `/workspace` to a worktree directory.
4. **`coast unassign`** — after reverting `/workspace` back to the project root.

The private directories persist across stop/start cycles (they live on the container's filesystem, not on the shared mount). On `coast rm`, they are destroyed along with the container.

## When to Use It

Use `private_paths` when a tool writes per-process or per-instance state into a workspace directory that conflicts across concurrent Coast instances:

- **File locks**: `.next/dev/lock`, Cargo's `target/.cargo-lock`, Gradle's `.gradle/lock`
- **Build caches**: `.next`, `.turbo`, `target/`, `.vite`
- **PID files**: any daemon that writes a PID file into the workspace

Do not use `private_paths` for data that needs to be shared across instances or visible on the host. If you need persistent, Docker-managed isolated data (like database volumes), use [volumes with `strategy = "isolated"`](../coastfiles/VOLUMES.md) instead.

## Validation Rules

- Paths must be relative (no leading `/`)
- Paths must not contain `..` components
- Paths must not overlap — listing both `frontend/.next` and `frontend/.next/cache` is an error because the first mount would shadow the second

## Relationship to Volumes

`private_paths` and `[volumes]` solve different isolation problems:

| | `private_paths` | `[volumes]` |
|---|---|---|
| **What** | Workspace-relative directories | Docker-managed named volumes |
| **Where** | Inside `/workspace` | Arbitrary container mount paths |
| **Backed by** | Container-local filesystem (`/coast-private/`) | Docker named volumes |
| **Isolation** | Always per-instance | `isolated` or `shared` strategy |
| **Survives `coast rm`** | No | Isolated: no. Shared: yes. |
| **Use case** | Build artifacts, lock files, caches | Databases, persistent application data |

## Configuration Reference

See [`private_paths`](../coastfiles/PROJECT.md) in the Coastfile reference for the full syntax and examples.
