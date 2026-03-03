#!/usr/bin/env bash
#
# Integration test for mixed service types (Docker Compose + bare process services).
#
# Tests the coast-mixed example which uses both compose = "docker-compose.yml"
# AND [services] to run a compose API server alongside a bare vite dev server.
#
# Prerequisites:
#   - Docker running
#   - socat installed (brew install socat)
#   - Coast binaries built (cargo build --release)
#
# Usage:
#   ./integrated-examples/test_mixed_service_types.sh

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
# Test 1: Build (mixed services project)
# ============================================================

echo ""
echo "=== Test 1: Build mixed services project ==="

BUILD_OUTPUT=$($COAST build 2>&1) || { echo "$BUILD_OUTPUT"; fail "coast build failed"; }
assert_contains "$BUILD_OUTPUT" "coast-mixed" "build output mentions project name"
pass "coast build succeeded for mixed services project"

# ============================================================
# Test 2: Run — both compose and bare services start
# ============================================================

echo ""
echo "=== Test 2: Run with mixed services ==="

RUN_OUTPUT=$($COAST run test-mixed 2>&1) || { echo "$RUN_OUTPUT"; fail "coast run failed"; }
pass "coast run test-mixed succeeded"

# Give services time to start
sleep 5

LS_OUTPUT=$($COAST ls 2>&1)
assert_contains "$LS_OUTPUT" "test-mixed" "instance listed"
assert_contains "$LS_OUTPUT" "running" "instance status is running"
pass "instance is running with mixed services"

# ============================================================
# Test 3: PS — shows both compose and bare services
# ============================================================

echo ""
echo "=== Test 3: PS shows both service types ==="

PS_OUTPUT=$($COAST ps test-mixed 2>&1) || { echo "$PS_OUTPUT"; fail "coast ps failed"; }
assert_contains "$PS_OUTPUT" "api" "ps shows compose api service"
assert_contains "$PS_OUTPUT" "vite" "ps shows bare vite service"
assert_contains "$PS_OUTPUT" "running" "at least one service is running"
pass "coast ps shows both compose and bare services"

# ============================================================
# Test 4: Logs — shows output from both service types
# ============================================================

echo ""
echo "=== Test 4: Logs from mixed services ==="

LOGS_OUTPUT=$($COAST logs test-mixed 2>&1) || { echo "$LOGS_OUTPUT"; fail "coast logs failed"; }
assert_contains "$LOGS_OUTPUT" "listening" "logs contain server startup message"
pass "coast logs shows mixed service output"

# ============================================================
# Test 5: Ports — both services have ports
# ============================================================

echo ""
echo "=== Test 5: Dynamic port access ==="

PORTS_OUTPUT=$($COAST ports test-mixed 2>&1) || { echo "$PORTS_OUTPUT"; fail "coast ports failed"; }
assert_contains "$PORTS_OUTPUT" "api" "ports shows compose api service"
assert_contains "$PORTS_OUTPUT" "vite" "ports shows bare vite service"
pass "coast ports shows both service types"

# Try to reach the vite bare service
VITE_PORT=$(extract_dynamic_port "$RUN_OUTPUT" "vite")
if [ -n "$VITE_PORT" ]; then
  sleep 1
  CURL_VITE=$(curl -s --max-time 5 "http://localhost:${VITE_PORT}" 2>&1) || true
  if echo "$CURL_VITE" | grep -q "vite"; then
    pass "vite bare service reachable via dynamic port ${VITE_PORT}"
  else
    echo "curl output: $CURL_VITE"
    fail "vite bare service not reachable via dynamic port ${VITE_PORT}"
  fi
else
  fail "could not extract dynamic port for vite service"
fi

# ============================================================
# Test 6: Exec still works
# ============================================================

echo ""
echo "=== Test 6: Exec into mixed services instance ==="

EXEC_OUTPUT=$($COAST exec test-mixed -- node --version 2>&1) || { echo "$EXEC_OUTPUT"; fail "exec failed"; }
assert_contains "$EXEC_OUTPUT" "v" "node version output"
pass "coast exec works in mixed services instance"

# ============================================================
# Test 7: Restart services — both types restart
# ============================================================

echo ""
echo "=== Test 7: Restart services ==="

RESTART_OUTPUT=$($COAST restart-services test-mixed 2>&1) || { echo "$RESTART_OUTPUT"; fail "coast restart-services failed"; }
pass "coast restart-services succeeded"

sleep 5

PS_AFTER=$($COAST ps test-mixed 2>&1) || { echo "$PS_AFTER"; fail "ps after restart failed"; }
assert_contains "$PS_AFTER" "api" "api still present after restart"
assert_contains "$PS_AFTER" "vite" "vite still present after restart"
pass "both service types present after restart"

# ============================================================
# Test 8: Stop and start
# ============================================================

echo ""
echo "=== Test 8: Stop and start ==="

STOP_OUTPUT=$($COAST stop test-mixed 2>&1) || { echo "$STOP_OUTPUT"; fail "coast stop failed"; }
pass "coast stop succeeded"

START_OUTPUT=$($COAST start test-mixed 2>&1) || { echo "$START_OUTPUT"; fail "coast start failed"; }
pass "coast start succeeded"

sleep 5

LS_OUTPUT2=$($COAST ls 2>&1)
assert_contains "$LS_OUTPUT2" "running" "instance is running after restart"
pass "instance running after stop/start cycle"

# ============================================================
# Test 9: Remove
# ============================================================

echo ""
echo "=== Test 9: Remove instance ==="

RM_OUTPUT=$($COAST rm test-mixed 2>&1) || { echo "$RM_OUTPUT"; fail "coast rm failed"; }
pass "coast rm succeeded"

LS_OUTPUT3=$($COAST ls 2>&1)
assert_not_contains "$LS_OUTPUT3" "test-mixed" "instance removed from listing"

REMAINING=$(docker ps -a --filter "label=coast.managed=true" --format '{{.Names}}' 2>/dev/null | grep "coast-mixed" || true)
if [ -z "$REMAINING" ]; then
  pass "no managed containers remain"
else
  fail "managed containers still exist: $REMAINING"
fi

# ============================================================
# Summary
# ============================================================

echo ""
echo "=== All mixed service types tests passed! ==="
