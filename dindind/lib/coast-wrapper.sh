#!/bin/bash
# Wrapper around the coast binary for DinDinD integration tests.
# After a successful `coast run`, sets up socat egress forwarders inside the
# Coast DinD container so inner compose services can reach host services
# through host.docker.internal.
REAL_COAST="${REAL_COAST:-/tmp/coast}"

if [ ! -x "$REAL_COAST" ]; then
  echo "coast-wrapper: real binary not found at $REAL_COAST" >&2
  exit 1
fi

# For non-run commands, pass through directly
case "${1:-}" in
  run) ;;
  *)   exec "$REAL_COAST" "$@" ;;
esac

# For `coast run`, run the real binary and capture its exit code
"$REAL_COAST" "$@"
rc=$?

if [ $rc -ne 0 ]; then
  exit $rc
fi

instance_name="${2:-}"
if [ -z "$instance_name" ]; then
  exit $rc
fi

# Parse egress ports from the Coastfile in the current directory
if [ ! -f Coastfile ]; then
  exit $rc
fi

egress_ports=()
egress_section=false
while IFS= read -r line; do
  case "$line" in
    "[egress]"*) egress_section=true; continue ;;
    "["*)        egress_section=false; continue ;;
  esac
  if [ "$egress_section" = true ]; then
    port=$(echo "$line" | grep -oE '[0-9]+' | head -1)
    [ -n "$port" ] && egress_ports+=("$port")
  fi
done < Coastfile

if [ ${#egress_ports[@]} -eq 0 ]; then
  exit $rc
fi

# Install socat inside the Coast DinD container (docker:dind is Alpine-based)
"$REAL_COAST" exec "$instance_name" -- \
  sh -c "command -v socat >/dev/null 2>&1 || apk add --no-cache socat >/dev/null 2>&1" \
  2>/dev/null || true

# Set up socat forwarders inside the Coast DinD container for each egress port.
for port in "${egress_ports[@]}"; do
  "$REAL_COAST" exec "$instance_name" -- \
    sh -c "nohup socat TCP-LISTEN:${port},fork,reuseaddr TCP:host.docker.internal:${port} >/dev/null 2>&1 &" \
    2>/dev/null || true
done

# Give socat a moment to bind
sleep 1

exit $rc
