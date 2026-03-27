#!/usr/bin/env bash
#
# Integration test: coastd process singleton enforcement.
#
# Verifies that only one coastd process can run at a time:
#   1. Starting a second coastd while one is running is rejected
#   2. daemon restart produces exactly one coastd process
#   3. flock is released on crash (SIGKILL), allowing a new daemon to start
#   4. Stale PID file after crash doesn't block a new daemon
#
# Uses an isolated COAST_HOME (temp dir) and custom ports to avoid
# interfering with a real daemon.
#
# Prerequisites:
#   - Docker running
#   - Coast binaries built (cargo build --release)
#
# Usage:
#   ./integrated-examples/test_coastd_process_singleton.sh

set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/helpers.sh"

ORIGINAL_HOME="${HOME:-}"
ORIGINAL_PATH="${PATH:-}"
ORIGINAL_COAST_HOME="${COAST_HOME:-}"
RELEASE_COAST="$COAST"
RELEASE_COASTD="$COASTD"

TEST_HOME=""
TEST_BIN_DIR=""
TEST_API_PORT="31419"
TEST_DNS_PORT="5358"

cleanup() {
    echo ""
    echo "--- Cleaning up ---"

    pkill -f "coastd-singleton-test" 2>/dev/null || true
    sleep 1

    if [ -n "$TEST_HOME" ]; then
        rm -rf "$TEST_HOME" 2>/dev/null || true
    fi

    export HOME="$ORIGINAL_HOME"
    export PATH="$ORIGINAL_PATH"
    if [ -n "$ORIGINAL_COAST_HOME" ]; then
        export COAST_HOME="$ORIGINAL_COAST_HOME"
    else
        unset COAST_HOME 2>/dev/null || true
    fi

    echo "Cleanup complete."
}

trap cleanup EXIT

echo "=== test_coastd_process_singleton.sh — singleton enforcement ==="
echo ""

TEST_HOME="$(mktemp -d)"
TEST_BIN_DIR="$TEST_HOME/bin"
mkdir -p "$TEST_BIN_DIR"

# Use the real binary, not the DinD wrapper
COAST_BIN="${REAL_COAST:-$RELEASE_COAST}"
COASTD_BIN="${REAL_COASTD:-$RELEASE_COASTD}"

# Give the test binary a unique name so cleanup pkill is precise.
# Also create a "coastd" symlink so `coast daemon restart` can find it.
cp "$COAST_BIN" "$TEST_BIN_DIR/coast"
cp "$COASTD_BIN" "$TEST_BIN_DIR/coastd-singleton-test"
ln -sf "$TEST_BIN_DIR/coastd-singleton-test" "$TEST_BIN_DIR/coastd"
chmod +x "$TEST_BIN_DIR/coast" "$TEST_BIN_DIR/coastd-singleton-test"

export HOME="$TEST_HOME"
export PATH="$TEST_BIN_DIR:$ORIGINAL_PATH"
export COAST_HOME="$TEST_HOME/.coast"
export COAST_API_PORT="$TEST_API_PORT"
export COAST_DNS_PORT="$TEST_DNS_PORT"
COAST="$TEST_BIN_DIR/coast"
COASTD="$TEST_BIN_DIR/coastd-singleton-test"

preflight_checks

count_coastd_procs() {
    local count
    # Match our unique binary name OR the symlink name, excluding the coast CLI
    count=$(ps aux 2>/dev/null | grep -E "coastd-singleton-test|${TEST_BIN_DIR}/coastd" | grep -v grep | grep -v "/coast " | wc -l | tr -d ' ') || true
    echo "${count:-0}"
}

# ============================================================
# Test 1: Second coastd is rejected while first is running
# ============================================================

echo ""
echo "=== Test 1: Second coastd rejected while first is running ==="

mkdir -p "$COAST_HOME"
"$COASTD" --foreground &>/tmp/coastd-singleton-1.log &
DAEMON1_PID=$!
sleep 2

kill -0 "$DAEMON1_PID" 2>/dev/null || fail "first daemon did not start"
pass "first daemon running (PID $DAEMON1_PID)"

PID_FILE_CONTENT=$(cat "$COAST_HOME/coastd.pid" 2>/dev/null | tr -d '[:space:]')
[ -n "$PID_FILE_CONTENT" ] || fail "PID file not written"
pass "PID file written: $PID_FILE_CONTENT"

# Try to start a second daemon directly (bypassing CLI "already running" guard)
"$COASTD" --foreground &>/tmp/coastd-singleton-2.log &
DAEMON2_PID=$!
sleep 2

# Check if the second daemon is still alive
if kill -0 "$DAEMON2_PID" 2>/dev/null; then
    PROC_COUNT=$(count_coastd_procs)
    echo "  WARNING: second daemon is alive (PID $DAEMON2_PID), total procs: $PROC_COUNT"
    echo "  This is the bug we are fixing -- two coastd processes are running."
    # Kill both for cleanup
    kill "$DAEMON2_PID" 2>/dev/null || true
    kill "$DAEMON1_PID" 2>/dev/null || true
    sleep 1
    fail "second coastd should have been rejected but it started successfully"
else
    pass "second coastd was rejected (exited immediately)"
fi

SECOND_LOG=$(cat /tmp/coastd-singleton-2.log 2>/dev/null || true)
assert_contains "$SECOND_LOG" "already running" "second daemon log mentions another instance"

# Verify only 1 daemon process
PROC_COUNT=$(count_coastd_procs)
assert_eq "$PROC_COUNT" "1" "exactly 1 coastd process running"

# Clean up daemon 1
kill "$DAEMON1_PID" 2>/dev/null || true
wait "$DAEMON1_PID" 2>/dev/null || true
sleep 1
rm -f "$COAST_HOME/coastd.pid" "$COAST_HOME/coastd.sock" "$COAST_HOME/coastd.lock"

# ============================================================
# Test 2: daemon restart produces exactly one process
# ============================================================

echo ""
echo "=== Test 2: daemon restart produces exactly one process ==="

"$COASTD" --foreground &>/tmp/coastd-singleton-3.log &
DAEMON3_PID=$!
sleep 2
kill -0 "$DAEMON3_PID" 2>/dev/null || fail "daemon did not start for test 2"
pass "daemon started for restart test (PID $DAEMON3_PID)"

"$COAST" daemon restart 2>&1 || fail "coast daemon restart failed"
sleep 2

PROC_COUNT=$(count_coastd_procs)
# After restart, should have exactly 1 (the new one)
[ "$PROC_COUNT" -le 1 ] || fail "expected at most 1 coastd after restart, got $PROC_COUNT"
pass "at most 1 coastd process after restart"

# Verify the new daemon is functional
STATUS=$("$COAST" daemon status 2>&1 || true)
assert_contains "$STATUS" "is running" "daemon is running after restart"

# Get new PID
NEW_PID=$(cat "$COAST_HOME/coastd.pid" 2>/dev/null | tr -d '[:space:]')
[ -n "$NEW_PID" ] || fail "PID file should exist after restart"
[ "$NEW_PID" != "$DAEMON3_PID" ] || fail "PID should change after restart"
pass "new daemon PID ($NEW_PID) differs from old ($DAEMON3_PID)"

# Kill for next test
"$COAST" daemon kill 2>&1 || true
sleep 1
rm -f "$COAST_HOME/coastd.lock"

# ============================================================
# Test 3: flock released on crash (SIGKILL)
# ============================================================

echo ""
echo "=== Test 3: flock released on crash ==="

"$COASTD" --foreground &>/tmp/coastd-singleton-4.log &
DAEMON4_PID=$!
sleep 2
kill -0 "$DAEMON4_PID" 2>/dev/null || fail "daemon did not start for test 3"
pass "daemon started for crash test (PID $DAEMON4_PID)"

# SIGKILL -- unclean death, no cleanup handlers run
kill -9 "$DAEMON4_PID" 2>/dev/null
wait "$DAEMON4_PID" 2>/dev/null || true
sleep 1

# PID file and lock file should still exist (no cleanup on SIGKILL)
[ -f "$COAST_HOME/coastd.pid" ] || pass "PID file already cleaned (acceptable)"

# Start a new daemon -- flock should be released by kernel
"$COASTD" --foreground &>/tmp/coastd-singleton-5.log &
DAEMON5_PID=$!
sleep 2

if kill -0 "$DAEMON5_PID" 2>/dev/null; then
    pass "new daemon started after crash (PID $DAEMON5_PID)"
else
    cat /tmp/coastd-singleton-5.log 2>/dev/null || true
    fail "new daemon should start after crash (flock should be released)"
fi

PROC_COUNT=$(count_coastd_procs)
assert_eq "$PROC_COUNT" "1" "exactly 1 coastd after crash recovery"

kill "$DAEMON5_PID" 2>/dev/null || true
wait "$DAEMON5_PID" 2>/dev/null || true
sleep 1
rm -f "$COAST_HOME/coastd.pid" "$COAST_HOME/coastd.sock" "$COAST_HOME/coastd.lock"

# ============================================================
# Test 4: Stale PID file doesn't block new daemon
# ============================================================

echo ""
echo "=== Test 4: Stale PID file doesn't block new daemon ==="

# Write a fake PID file pointing to a non-existent process
mkdir -p "$COAST_HOME"
echo "99999" > "$COAST_HOME/coastd.pid"
pass "wrote stale PID file (PID 99999)"

"$COASTD" --foreground &>/tmp/coastd-singleton-6.log &
DAEMON6_PID=$!
sleep 2

if kill -0 "$DAEMON6_PID" 2>/dev/null; then
    pass "daemon started despite stale PID file (PID $DAEMON6_PID)"
else
    cat /tmp/coastd-singleton-6.log 2>/dev/null || true
    fail "daemon should start despite stale PID file"
fi

REAL_PID=$(cat "$COAST_HOME/coastd.pid" 2>/dev/null | tr -d '[:space:]')
[ "$REAL_PID" != "99999" ] || fail "PID file should be overwritten with real PID"
pass "PID file updated to real PID ($REAL_PID)"

kill "$DAEMON6_PID" 2>/dev/null || true
wait "$DAEMON6_PID" 2>/dev/null || true

echo ""
echo "==========================================="
echo "  ALL SINGLETON TESTS PASSED"
echo "==========================================="
