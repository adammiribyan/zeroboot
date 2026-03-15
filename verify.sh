#!/bin/bash
# Zeroboot Verification Test Suite
# Verifies benchmark claims with pass/fail assertions.
# Usage: sudo ./verify.sh
set -uo pipefail

# Need high fd limit for 1000-concurrent test in bench
ulimit -n 65535 2>/dev/null || true

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/zeroboot"
WORKDIR_C="$SCRIPT_DIR/workdir"
WORKDIR_PYTHON="$SCRIPT_DIR/workdir-python"
WORKDIR_NODE="$SCRIPT_DIR/workdir-node"

PASS=0
FAIL=0
TOTAL=0
RESULTS=()

pass() {
    RESULTS+=("[PASS] $1")
    ((PASS++))
    ((TOTAL++))
}

fail() {
    RESULTS+=("[FAIL] $1")
    ((FAIL++))
    ((TOTAL++))
}

echo "============================================"
echo "  Zeroboot Verification Suite"
echo "============================================"
echo

# ── Step 1: Build ───────────────────────────────────────────────────
echo "[1/6] Building release binary..."
cd "$SCRIPT_DIR"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
cargo build --release 2>&1 | tail -1
echo

# ── Step 2: Check prerequisites ─────────────────────────────────────
echo "[2/6] Checking prerequisites..."
for dir in "$WORKDIR_C" "$WORKDIR_PYTHON" "$WORKDIR_NODE"; do
    if [ ! -f "$dir/snapshot/mem" ] || [ ! -f "$dir/snapshot/vmstate" ]; then
        echo "ERROR: Snapshot missing in $dir. Create templates first (see CLAUDE.md)."
        exit 1
    fi
done
echo "  All snapshots present."

# Quick smoke test
OUTPUT=$("$BINARY" test-exec "$WORKDIR_PYTHON" "print('template_ok')" 2>/dev/null) || true
if echo "$OUTPUT" | grep -q "template_ok"; then
    echo "  Python template OK."
else
    echo "ERROR: Python template broken. Output: $OUTPUT"
    exit 1
fi
echo

# ── Step 3: Fork latency + concurrent + isolation (via bench) ───────
echo "[3/6] Running fork benchmarks (1000 fork iterations, 100 fork+exec, concurrent, isolation)..."
echo "  This takes ~60 seconds..."
"$BINARY" bench "$WORKDIR_C" > /tmp/zb_verify_bench.txt 2>&1 || true
BENCH_OUT=$(cat /tmp/zb_verify_bench.txt)

# Parse fork latency: "    P50:     713.3 µs (0.713 ms)"
# Extract from "Full fork" section
FORK_P50=$(echo "$BENCH_OUT" | grep -A6 "Full fork" | grep "P50:" | grep -oP '\([\d.]+ ms\)' | grep -oP '[\d.]+' || echo "")
FORK_P99=$(echo "$BENCH_OUT" | grep -A8 "Full fork" | grep "P99:" | grep -oP '\([\d.]+ ms\)' | grep -oP '[\d.]+' || echo "")

FORK_P50_THRESH=2
FORK_P99_THRESH=5
if [ -n "$FORK_P50" ] && awk "BEGIN {exit !($FORK_P50 < $FORK_P50_THRESH)}"; then
    pass "Fork latency p50: ${FORK_P50}ms (threshold: <${FORK_P50_THRESH}ms)"
else
    fail "Fork latency p50: ${FORK_P50:-?}ms (threshold: <${FORK_P50_THRESH}ms)"
fi
if [ -n "$FORK_P99" ] && awk "BEGIN {exit !($FORK_P99 < $FORK_P99_THRESH)}"; then
    pass "Fork latency p99: ${FORK_P99}ms (threshold: <${FORK_P99_THRESH}ms)"
else
    fail "Fork latency p99: ${FORK_P99:-?}ms (threshold: <${FORK_P99_THRESH}ms)"
fi

# Parse concurrent forks: "  100 concurrent: 60.0ms total"
CONCURRENT_MS=$(echo "$BENCH_OUT" | grep "100 concurrent" | head -1 | grep -oP '[\d.]+ms total' | grep -oP '[\d.]+' || echo "")
CONCURRENT_THRESH=500
if [ -n "$CONCURRENT_MS" ] && awk "BEGIN {exit !($CONCURRENT_MS < $CONCURRENT_THRESH)}"; then
    pass "Concurrent 100 forks: ${CONCURRENT_MS}ms (threshold: <${CONCURRENT_THRESH}ms)"
else
    fail "Concurrent 100 forks: ${CONCURRENT_MS:-?}ms (threshold: <${CONCURRENT_THRESH}ms)"
fi

# Parse isolation test
ISOLATION_PASS=true
echo "$BENCH_OUT" | grep -q "PASS: Isolation verified" || ISOLATION_PASS=false
echo "$BENCH_OUT" | grep -q "PASS: Bidirectional isolation OK" || ISOLATION_PASS=false
if $ISOLATION_PASS; then
    pass "Memory isolation: secret not visible across forks"
else
    fail "Memory isolation: secret leaked across forks"
fi
echo

# ── Step 4: Language tests ──────────────────────────────────────────
echo "[4/6] Running language tests..."

# Helper: run code and return cleaned stdout (10s timeout per test)
# Guest echoes the command back before output, so we strip:
#   1. Everything before "=== Output ==="
#   2. The "=== Output ===" line itself
#   3. The echoed command line (first non-empty line)
#   4. ZEROBOOT_DONE marker
run_test() {
    local workdir="$1"
    local code="$2"
    local raw
    raw=$(timeout 10 "$BINARY" test-exec "$workdir" "$code" 2>/dev/null) || true
    echo "$raw" \
        | sed -n '/=== Output ===/,$ p' \
        | tail -n +2 \
        | sed 's/ZEROBOOT_DONE//g' \
        | sed 's/[[:space:]]*$//' \
        | sed '/^$/d' \
        | tail -n +2
}

# Python print(2+2)
OUT=$(run_test "$WORKDIR_PYTHON" "print(2+2)")
if echo "$OUT" | grep -q "^4$"; then
    pass "Python print(2+2) = 4"
else
    fail "Python print(2+2) = 4 (got: $(echo "$OUT" | tr '\n' ' '))"
fi

# numpy array multiplication
OUT=$(run_test "$WORKDIR_PYTHON" "print(numpy.array([1.5,2.5]) * 2)")
if echo "$OUT" | grep -q '\[3\. 5\.\]'; then
    pass "numpy.array([1.5,2.5]) * 2 = [3. 5.]"
else
    fail "numpy.array([1.5,2.5]) * 2 = [3. 5.] (got: $(echo "$OUT" | tr '\n' ' '))"
fi

# numpy random returns 3 floats
OUT=$(run_test "$WORKDIR_PYTHON" "print(numpy.random.rand(3))")
if echo "$OUT" | grep -qP '^\[[\d.]+ [\d.]+ [\d.]+\]$'; then
    pass "numpy.random.rand(3) returns 3 floats"
else
    fail "numpy.random.rand(3) returns 3 floats (got: $(echo "$OUT" | tr '\n' ' '))"
fi

# numpy dot product (validates float math end-to-end)
OUT=$(run_test "$WORKDIR_PYTHON" "print(numpy.dot([1,2,3],[4,5,6]))")
if echo "$OUT" | grep -q "^32$"; then
    pass "numpy.dot([1,2,3],[4,5,6]) = 32"
else
    fail "numpy.dot([1,2,3],[4,5,6]) = 32 (got: $(echo "$OUT" | tr '\n' ' '))"
fi

# Node.js
OUT=$(run_test "$WORKDIR_NODE" "console.log(1+1)")
if echo "$OUT" | grep -q "^2$"; then
    pass "Node.js console.log(1+1) = 2"
else
    fail "Node.js console.log(1+1) = 2 (got: $(echo "$OUT" | tr '\n' ' '))"
fi
echo

# ── Step 5: Summary ─────────────────────────────────────────────────
echo "[5/6] Results"
echo
for r in "${RESULTS[@]}"; do
    echo "  $r"
done
echo
echo "  ${PASS}/${TOTAL} tests passed"
echo

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
