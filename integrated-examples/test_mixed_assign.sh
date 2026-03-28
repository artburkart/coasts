#!/usr/bin/env bash
#
# Integration test: bare services restart after assign in mixed projects.
#
# Reproduces the bug where bare services with assign.services = "restart"
# go down after coast assign and stay down. Uses coast-mixed which has
# both compose and bare services with [assign] default = "none" and
# per-service [assign.services] overrides.
#
# Prerequisites:
#   - Docker running
#   - socat installed (brew install socat)
#   - Coast binaries built (cargo build --release)
#
# Usage:
#   ./integrated-examples/test_mixed_assign.sh

set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/helpers.sh"

register_cleanup

# --- Preflight ---

preflight_checks

# --- Setup ---

echo ""
echo "=== Setup ==="

clean_slate

"$HELPERS_DIR/setup.sh"
pass "Examples initialized"

cd "$PROJECTS_DIR/coast-mixed"

start_daemon

# ============================================================
# Test 1: Build and run
# ============================================================

echo ""
echo "=== Test 1: Build and run ==="

BUILD_OUTPUT=$($COAST build 2>&1) || { echo "$BUILD_OUTPUT"; fail "coast build failed"; }
pass "coast build succeeded"

RUN_OUTPUT=$($COAST run dev-1 2>&1) || { echo "$RUN_OUTPUT"; fail "coast run failed"; }
CLEANUP_INSTANCES+=("dev-1")
pass "coast run dev-1 succeeded"

# Extract both ports
API_PORT=$(extract_dynamic_port "$RUN_OUTPUT" "api")
VITE_PORT=$(extract_dynamic_port "$RUN_OUTPUT" "vite")
[ -n "$API_PORT" ] || fail "Could not extract api dynamic port"
[ -n "$VITE_PORT" ] || fail "Could not extract vite dynamic port"
pass "api port: $API_PORT, vite port: $VITE_PORT"

# Wait for both services
wait_for_healthy "$API_PORT" 60 || fail "api service not healthy"
pass "compose api service healthy"

# Wait for bare service to start (may need install step + startup time)
VITE_HEALTHY=false
for i in $(seq 1 15); do
    VITE_RESP=$(curl -sf "http://localhost:${VITE_PORT}/" 2>&1) && { VITE_HEALTHY=true; break; }
    sleep 2
done
if [ "$VITE_HEALTHY" = "false" ]; then
    echo "  vite did not respond within 30s"
    fail "vite bare service not healthy"
fi
assert_contains "$VITE_RESP" "vite" "vite bare service responds"
pass "bare vite service healthy"

# ============================================================
# Test 2: Assign to feature-v2
# ============================================================

echo ""
echo "=== Test 2: Assign to feature-v2 ==="

ASSIGN_OUT=$($COAST assign dev-1 --worktree feature-v2 2>&1) || { echo "$ASSIGN_OUT"; fail "coast assign failed"; }
assert_contains "$ASSIGN_OUT" "Assigned worktree" "assign to feature-v2 succeeded"
pass "assigned to feature-v2"

# ============================================================
# Test 3: Verify bare service restarted after assign
# ============================================================

echo ""
echo "=== Test 3: Bare service responds after assign ==="

# Give bare service time to restart (install + start)
VITE_UP=false
for i in $(seq 1 15); do
    VITE_AFTER=$(curl -sf "http://localhost:${VITE_PORT}/" 2>&1) && { VITE_UP=true; break; }
    sleep 2
done

if [ "$VITE_UP" = "false" ]; then
    echo "  BARE SERVICE IS DOWN after assign -- this is the bug"
    echo "  vite did not respond within 30s after assign"
    fail "bare vite service is down after assign (should have restarted)"
fi

echo "  vite response: $VITE_AFTER"
assert_contains "$VITE_AFTER" "v2" "vite serves feature-v2 code after assign"
pass "bare service restarted with v2 code after assign"

# ============================================================
# Test 4: Verify compose service still healthy
# ============================================================

echo ""
echo "=== Test 4: Compose service still healthy after assign ==="

API_AFTER=$(curl -sf "http://localhost:${API_PORT}/" 2>&1 || echo '{"error":"no response"}')
assert_contains "$API_AFTER" "api" "compose api still responds after assign"
pass "compose service survived assign"

# ============================================================
# Cleanup
# ============================================================

echo ""
echo "=== Cleanup ==="

$COAST rm dev-1 2>&1 || fail "coast rm failed"
CLEANUP_INSTANCES=()

echo ""
echo "==========================================="
echo "  ALL MIXED ASSIGN TESTS PASSED"
echo "==========================================="
