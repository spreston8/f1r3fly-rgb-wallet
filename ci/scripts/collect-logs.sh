#!/bin/bash
set -e

# Script to collect logs and artifacts for CI debugging
# Usage: ./collect-logs.sh <output-dir>

OUTPUT_DIR="${1:-ci-logs}"
COMPOSE_FILE="${2:-ci/docker-compose.yml}"

echo "📦 Collecting logs and artifacts to: $OUTPUT_DIR"

# Create output directory structure
mkdir -p "$OUTPUT_DIR/docker"
mkdir -p "$OUTPUT_DIR/services"
mkdir -p "$OUTPUT_DIR/wallet"

# Collect docker compose logs
echo "  → Docker compose logs..."
docker compose -f $COMPOSE_FILE logs > "$OUTPUT_DIR/docker/compose-all.log" 2>&1 || echo "No compose logs available"
docker compose -f $COMPOSE_FILE logs bitcoind > "$OUTPUT_DIR/services/bitcoind.log" 2>&1 || echo "No bitcoind logs"
docker compose -f $COMPOSE_FILE logs electrs > "$OUTPUT_DIR/services/electrs.log" 2>&1 || echo "No electrs logs"
docker compose -f $COMPOSE_FILE logs f1r3node > "$OUTPUT_DIR/services/f1r3node.log" 2>&1 || echo "No f1r3node logs"

# Collect container stats
echo "  → Container stats..."
docker compose -f $COMPOSE_FILE ps --format json > "$OUTPUT_DIR/docker/containers.json" 2>&1 || echo "[]"

# Collect service health status
echo "  → Service health..."
{
    echo "=== Bitcoin Core Status ==="
    docker compose -f $COMPOSE_FILE exec -T bitcoind \
        bitcoin-cli -regtest -rpcuser=user -rpcpassword=password getblockchaininfo 2>&1 || echo "Bitcoin RPC not available"
    
    echo ""
    echo "=== Electrs Status ==="
    curl -s http://localhost:3002/blocks/tip/height 2>&1 || echo "Electrs API not available"
    
    echo ""
    echo "=== F1r3node Status ==="
    curl -s http://localhost:40403/api/status 2>&1 || echo "F1r3node API not available"
} > "$OUTPUT_DIR/services/health-check.txt"

# Collect F1r3node specific artifacts (if available)
echo "  → F1r3node artifacts..."
docker compose -f $COMPOSE_FILE exec -T f1r3node \
    cat /var/lib/rnode/rnode.log > "$OUTPUT_DIR/services/f1r3node-detailed.log" 2>&1 || echo "F1r3node detailed log not available"

# Collect wallet test data (if exists)
echo "  → Wallet test data..."
if [ -d "$HOME/.f1r3fly-rgb-wallet" ]; then
    cp -r "$HOME/.f1r3fly-rgb-wallet" "$OUTPUT_DIR/wallet/" 2>/dev/null || echo "No wallet data"
fi

# Collect test temporary directories (if they exist)
if [ -d "/tmp/f1r3fly-test-*" ]; then
    echo "  → Test temporary directories..."
    cp -r /tmp/f1r3fly-test-* "$OUTPUT_DIR/wallet/" 2>/dev/null || echo "No test temp dirs"
fi

# Create summary
echo "  → Creating summary..."
{
    echo "=== Log Collection Summary ==="
    echo "Timestamp: $(date -u +"%Y-%m-%d %H:%M:%S UTC")"
    echo ""
    echo "=== Files Collected ==="
    find "$OUTPUT_DIR" -type f -exec echo "  {}" \; | sort
    echo ""
    echo "=== Total Size ==="
    du -sh "$OUTPUT_DIR"
} > "$OUTPUT_DIR/summary.txt"

echo "✅ Log collection complete: $OUTPUT_DIR"
echo "📊 Summary:"
cat "$OUTPUT_DIR/summary.txt"

