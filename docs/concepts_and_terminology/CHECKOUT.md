# Checkout

Checkout controls which Coast instance owns your project's [canonical ports](PORTS.md). When you check out a Coast, `localhost:3000`, `localhost:5432`, and every other canonical port maps straight to that instance.

```bash
coast checkout dev-1
```

```text
Before checkout:
  localhost:3000  ──→  (nothing)
  localhost:5432  ──→  (nothing)

After checkout:
  localhost:3000  ──→  dev-1 web
  localhost:5432  ──→  dev-1 db
```

Switching checkout is instant — Coast kills and respawns lightweight `socat` forwarders. No containers are restarted.

```bash
coast checkout dev-2   # instant swap

# localhost:3000  ──→  dev-2 web
# localhost:5432  ──→  dev-2 db
```

## Linux Note

Dynamic ports always work on Linux without special privileges.

Canonical ports below `1024` are different. If your Coastfile declares ports like `80` or `443`, Linux may block `coast checkout` from binding them until you configure the host. The usual fixes are:

- raise `net.ipv4.ip_unprivileged_port_start`
- grant bind capability to the forwarding binary or process

Coast reports this explicitly when the host denies the bind.

On WSL, Coast uses Docker-published checkout bridges so Windows browsers and tools can reach checked-out canonical ports through `127.0.0.1`, similar to Docker Desktop workflows like Sail.

For local HTTPS projects that use Caddy, Coast reuses one Caddy local CA per Coast installation. After you trust that root once, recreated workspaces under the same install keep using it.

The root certificate lives at:

- `~/.coast/caddy/pki/authorities/local/root.crt` for the regular install
- `~/.coast-dev/caddy/pki/authorities/local/root.crt` for `coast-dev`

Those are intentionally separate, so trusting `coast-dev` does not also trust a regular `coast` install, and vice versa.

To inspect or export the active install's root certificate:

```bash
coast cert info
coast cert path
coast cert fingerprint
coast cert export --to ~/Downloads/coast-root.crt
```

Coast leaves trust installation up to you. Export the cert, then import it into your OS or browser trust store as needed.

## Do You Need to Check Out?

Not necessarily. Every running Coast always has its own dynamic ports, and you can access any Coast through those ports at any time without checking anything out.

```bash
coast ports dev-1

# SERVICE    CANONICAL  DYNAMIC
# ★ web      3000       62217
#   db       5432       55681
```

You can open `localhost:62217` in your browser to hit dev-1's web server without checking it out. This is perfectly fine for many workflows, and you can run as many Coasts as you want without ever using `coast checkout`.

## When Checkout Is Useful

There are situations where dynamic ports are not enough and you need canonical ports:

- **Client applications hardcoded to canonical ports.** If you have a client running outside the Coast — a frontend dev server on your host, a mobile app on your phone, or a desktop app — that expects `localhost:3000` or `localhost:8080`, changing port numbers everywhere is impractical. Checking out the Coast gives you the real ports without changing any configuration.

- **Webhooks and callback URLs.** Services like Stripe, GitHub, or OAuth providers send callbacks to a URL you registered — usually something like `https://your-ngrok-tunnel.io` that forwards to `localhost:3000`. If you switch to a dynamic port, the callbacks stop arriving. Checking out ensures the canonical port is active for the Coast you are testing.

- **Database tools, debuggers, and IDE integrations.** Many GUI clients (pgAdmin, DataGrip, TablePlus), debuggers, and IDE run configurations save connection profiles with a specific port. Checkout lets you keep your saved profiles and just swap which Coast is behind them — no reconfiguring your debugger attach target or database connection every time you switch contexts.

## Releasing Checkout

If you want to release the canonical ports without checking out a different Coast:

```bash
coast checkout --none
```

After this, no Coast owns the canonical ports. All Coasts remain accessible through their dynamic ports.

## Only One at a Time

Exactly one Coast can be checked out at a time. If `dev-1` is checked out and you run `coast checkout dev-2`, the canonical ports instantly swap to `dev-2`. There is no gap — the old forwarders are killed and new ones are spawned in the same operation.

```text
┌──────────────────────────────────────────────────┐
│  Your machine                                    │
│                                                  │
│  Canonical (checked-out Coast only):             │
│    localhost:3000 ──→ dev-2 web                  │
│    localhost:5432 ──→ dev-2 db                   │
│                                                  │
│  Dynamic (always available):                     │
│    localhost:62217 ──→ dev-1 web                 │
│    localhost:55681 ──→ dev-1 db                  │
│    localhost:63104 ──→ dev-2 web                 │
│    localhost:57220 ──→ dev-2 db                  │
└──────────────────────────────────────────────────┘
```

Dynamic ports are unaffected by checkout. The only thing that changes is where the canonical ports point.
