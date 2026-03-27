#!/usr/bin/env bash
#
# Integration test: private_paths + bare services + assign/unassign.
#
# Reproduces and verifies fixes for two bugs:
#   1. private_paths overlay not reapplied after assign/unassign (flock leak)
#   2. Bare services don't restart after unassign
#
# Uses coast-private-paths-bare (bare node server + private_paths=["data"],
# two branches: main and feature-v2).
#
# Prerequisites:
#   - Docker running
#   - socat installed (brew install socat)
#   - Coast binaries built (cargo build --release)
#
# Usage:
#   ./integrated-examples/test_private_paths_assign.sh

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

cd "$PROJECTS_DIR/coast-private-paths-bare"

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

sleep 5

DYN_PORT=$(extract_dynamic_port "$RUN_OUTPUT" "web")
[ -n "$DYN_PORT" ] || fail "Could not extract web dynamic port"
pass "Dynamic port: $DYN_PORT"

# Verify serving main
RESP=$(curl -sf "http://localhost:${DYN_PORT}/" 2>&1 || echo '{"error":"no response"}')
assert_contains "$RESP" '"version":"main"' "serves main branch initially"

# ============================================================
# Test 2: Verify bare service holds persistent flock
# ============================================================

echo ""
echo "=== Test 2: Verify bare service holds flock on private_paths ==="

LOCK_STATUS=$(curl -sf "http://localhost:${DYN_PORT}/lock-status" 2>&1 || echo '{"lock_held":false}')
assert_contains "$LOCK_STATUS" '"lock_held":true' "bare service holds flock on /workspace/data/app.lock"

# Write a marker to private data dir
$COAST exec dev-1 -- sh -c "echo pre-assign > /workspace/data/marker" 2>&1 \
    || fail "failed to write marker"
pass "wrote marker to /workspace/data/marker"

# ============================================================
# Test 3: Assign to feature-v2
# ============================================================

echo ""
echo "=== Test 3: Assign to feature-v2 ==="

ASSIGN_OUT=$($COAST assign dev-1 --worktree feature-v2 2>&1) || { echo "$ASSIGN_OUT"; fail "coast assign failed"; }
assert_contains "$ASSIGN_OUT" "Assigned worktree" "assign to feature-v2 succeeded"
pass "assigned to feature-v2"

sleep 5

# ============================================================
# Test 4: Verify private_paths overlay survived assign
# ============================================================

echo ""
echo "=== Test 4: Verify private_paths overlay after assign ==="

# The marker should still be there (private_paths data persists per-instance)
MARKER_AFTER=$($COAST exec dev-1 -- cat /workspace/data/marker 2>&1) \
    || fail "failed to read marker after assign"
assert_contains "$MARKER_AFTER" "pre-assign" "private data persists across assign"

# ============================================================
# Test 5: Verify bare service restarted with feature-v2 AND holds flock
# ============================================================

echo ""
echo "=== Test 5: Bare service serves feature-v2 and holds flock after assign ==="

RESP_V2=$(curl -sf "http://localhost:${DYN_PORT}/" 2>&1 || echo '{"error":"no response"}')
assert_contains "$RESP_V2" '"version":"v2"' "bare service serves v2 after assign"
assert_contains "$RESP_V2" '"branch":"feature-v2"' "bare service reports feature-v2 branch"

# The critical check: did the new server acquire the flock?
# If private_paths overlay leaked, the old server's flock persists and this fails.
LOCK_AFTER_ASSIGN=$(curl -sf "http://localhost:${DYN_PORT}/lock-status" 2>&1 || echo '{"lock_held":false}')
assert_contains "$LOCK_AFTER_ASSIGN" '"lock_held":true' "new bare service acquired flock after assign (no leak)"

# ============================================================
# Test 6: Unassign back to main
# ============================================================

echo ""
echo "=== Test 6: Unassign back to main ==="

UNASSIGN_OUT=$($COAST unassign dev-1 2>&1) || { echo "$UNASSIGN_OUT"; fail "coast unassign failed"; }
pass "unassigned back to main"

sleep 8

# ============================================================
# Test 7: Verify private_paths overlay survived unassign
# ============================================================

echo ""
echo "=== Test 7: Verify private_paths overlay after unassign ==="

MARKER_UNASSIGN=$($COAST exec dev-1 -- cat /workspace/data/marker 2>&1) \
    || fail "failed to read marker after unassign"
assert_contains "$MARKER_UNASSIGN" "pre-assign" "private data persists across unassign"

# ============================================================
# Test 8: Verify bare service restarted with main code AND holds flock
# ============================================================

echo ""
echo "=== Test 8: Bare service serves main and holds flock after unassign ==="

RESP_MAIN=$(curl -sf "http://localhost:${DYN_PORT}/" 2>&1 || echo '{"error":"no response"}')
assert_contains "$RESP_MAIN" '"version":"main"' "bare service serves main after unassign"
assert_contains "$RESP_MAIN" '"branch":"main"' "bare service reports main branch"

# The critical check: did the new server acquire the flock after unassign?
LOCK_AFTER_UNASSIGN=$(curl -sf "http://localhost:${DYN_PORT}/lock-status" 2>&1 || echo '{"lock_held":false}')
assert_contains "$LOCK_AFTER_UNASSIGN" '"lock_held":true' "bare service acquired flock after unassign (no leak)"

# ============================================================
# Cleanup
# ============================================================

echo ""
echo "=== Cleanup ==="

$COAST rm dev-1 2>&1 || fail "coast rm failed"
CLEANUP_INSTANCES=()

echo ""
echo "==========================================="
echo "  ALL PRIVATE_PATHS + ASSIGN TESTS PASSED"
echo "==========================================="
