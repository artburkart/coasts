#!/usr/bin/env bash
#
# Integration test for private_paths filesystem isolation.
#
# Verifies that two Coast instances with private_paths = ["data"]
# get independent bind mounts so writes and flock locks don't collide.
#
# Prerequisites:
#   - Docker running
#   - socat installed (brew install socat)
#   - Coast binaries built (cargo build --release)
#
# Usage:
#   ./integrated-examples/test_private_paths.sh

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

cd "$PROJECTS_DIR/coast-private-paths"

start_daemon

# ============================================================
# Test 1: Build
# ============================================================

echo ""
echo "=== Test 1: Build private-paths project ==="

BUILD_OUTPUT=$($COAST build 2>&1) || { echo "$BUILD_OUTPUT"; fail "coast build failed"; }
assert_contains "$BUILD_OUTPUT" "coast-private-paths" "build output mentions project name"
pass "coast build succeeded"

# ============================================================
# Test 2: Run two instances
# ============================================================

echo ""
echo "=== Test 2: Run two instances ==="

RUN_OUTPUT_1=$($COAST run pp-1 2>&1) || { echo "$RUN_OUTPUT_1"; fail "coast run pp-1 failed"; }
pass "coast run pp-1 succeeded"

RUN_OUTPUT_2=$($COAST run pp-2 2>&1) || { echo "$RUN_OUTPUT_2"; fail "coast run pp-2 failed"; }
pass "coast run pp-2 succeeded"

sleep 3

LS_OUTPUT=$($COAST ls 2>&1)
assert_contains "$LS_OUTPUT" "pp-1" "pp-1 listed"
assert_contains "$LS_OUTPUT" "pp-2" "pp-2 listed"
pass "both instances visible in coast ls"

# ============================================================
# Test 3: Write isolation — each instance sees its own data/
# ============================================================

echo ""
echo "=== Test 3: Write isolation ==="

$COAST exec pp-1 -- sh -c "mkdir -p /workspace/data && echo instance1 > /workspace/data/marker" 2>&1 \
    || fail "failed to write marker in pp-1"

$COAST exec pp-2 -- sh -c "mkdir -p /workspace/data && echo instance2 > /workspace/data/marker" 2>&1 \
    || fail "failed to write marker in pp-2"

MARKER_1=$($COAST exec pp-1 -- cat /workspace/data/marker 2>&1) || fail "failed to read marker from pp-1"
MARKER_2=$($COAST exec pp-2 -- cat /workspace/data/marker 2>&1) || fail "failed to read marker from pp-2"

assert_contains "$MARKER_1" "instance1" "pp-1 sees its own marker"
assert_contains "$MARKER_2" "instance2" "pp-2 sees its own marker"
assert_not_contains "$MARKER_1" "instance2" "pp-1 does NOT see pp-2's marker"
assert_not_contains "$MARKER_2" "instance1" "pp-2 does NOT see pp-1's marker"
pass "write isolation verified — each instance has independent data/"

# ============================================================
# Test 4: flock isolation — concurrent locks don't conflict
# ============================================================

echo ""
echo "=== Test 4: flock isolation ==="

FLOCK_1=$($COAST exec pp-1 -- sh -c "flock -n /workspace/data/lockfile echo locked-1" 2>&1) \
    || fail "flock in pp-1 failed (should succeed)"
assert_contains "$FLOCK_1" "locked-1" "pp-1 acquired lock"

FLOCK_2=$($COAST exec pp-2 -- sh -c "flock -n /workspace/data/lockfile echo locked-2" 2>&1) \
    || fail "flock in pp-2 failed (should succeed — separate inode)"
assert_contains "$FLOCK_2" "locked-2" "pp-2 acquired lock"

pass "flock isolation verified — both instances lock the same relative path without conflict"

# ============================================================
# Test 5: Private paths backed by /coast-private/
# ============================================================

echo ""
echo "=== Test 5: Private paths backed by /coast-private/ ==="

PRIVATE_CHECK=$($COAST exec pp-1 -- cat /coast-private/data/marker 2>&1) \
    || fail "failed to read /coast-private/data/marker in pp-1"
assert_contains "$PRIVATE_CHECK" "instance1" "/coast-private/data has pp-1's marker"

PRIVATE_CHECK_2=$($COAST exec pp-2 -- cat /coast-private/data/marker 2>&1) \
    || fail "failed to read /coast-private/data/marker in pp-2"
assert_contains "$PRIVATE_CHECK_2" "instance2" "/coast-private/data has pp-2's marker"

pass "private paths backed by per-instance /coast-private/ storage"

# ============================================================
# Test 6: Cleanup
# ============================================================

echo ""
echo "=== Test 6: Remove instances ==="

$COAST rm pp-1 2>&1 || fail "coast rm pp-1 failed"
$COAST rm pp-2 2>&1 || fail "coast rm pp-2 failed"

LS_AFTER=$($COAST ls 2>&1)
assert_not_contains "$LS_AFTER" "pp-1" "pp-1 removed"
assert_not_contains "$LS_AFTER" "pp-2" "pp-2 removed"
pass "both instances removed"

echo ""
echo "=== All private_paths tests passed ==="
