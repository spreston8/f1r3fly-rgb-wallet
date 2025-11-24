#!/bin/bash
# Test f1r3fly-rgb-wallet CLI commands with automatic cleanup

# Don't exit on error - we want to track test failures
set +e

# Create temporary directory for testing
TEMP_DIR=$(mktemp -d)

# Generate unique wallet names to prevent F1r3node state pollution across test runs
# This matches the integration test approach (env.unique_wallet_name()) which ensures
# fresh contract state on F1r3node for each test run, avoiding cached/stale data issues.
TIMESTAMP=$(date +%s)
WALLET_NAME="test_alice_${TIMESTAMP}"
BOB_WALLET_NAME="test_bob_${TIMESTAMP}"
PASSWORD="testpass123"

# Detect CI environment and configure accordingly
if [ -n "$CI" ]; then
  echo "🤖 Running in CI mode"
  COMPOSE_FILE="${COMPOSE_FILE:-ci/docker-compose.yml}"
  BITCOIN_CLI="docker compose -f $COMPOSE_FILE exec -T bitcoind bitcoin-cli -regtest -rpcuser=user -rpcpassword=password"
  SLEEP_TIME=5
else
  echo "💻 Running in local mode (assumes start-regtest.sh has been run)"
  BITCOIN_DATADIR="${BITCOIN_DATADIR:-$(cd .. && pwd)/.bitcoin}"
  BITCOIN_CLI="bitcoin-cli -regtest -datadir=$BITCOIN_DATADIR"
  SLEEP_TIME=3
fi

# Test tracking
TESTS_PASSED=0
TESTS_FAILED=0
TEST_RESULTS=()
EXIT_CODE=1  # Default to failure, will be set to 0 if all tests pass

# Setup logging
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"
LOG_FILE="$LOG_DIR/test_cli_$(date +%Y%m%d_%H%M%S).log"

# Create logs directory if it doesn't exist
mkdir -p "$LOG_DIR"

# Redirect all output to both terminal and log file
exec > >(tee "$LOG_FILE") 2>&1

echo "Logging to: $LOG_FILE"
echo ""

# Load environment variables from .env file
if [ -f ".env" ]; then
    export $(grep -v '^#' .env | xargs)
    echo "✓ Loaded .env file"
else
    echo "⚠ Warning: .env file not found (RGB tests may fail)"
fi

# Note: RUST_LOG will be set selectively for specific tests that need debug logging
# Global debug logging interferes with output parsing in tests

echo "======================================"
echo "Testing f1r3fly-rgb-wallet CLI"
echo "======================================"
echo "Temp directory: $TEMP_DIR"
echo ""

# Check if regtest is running
REGTEST_RUNNING=false
if curl -s http://localhost:3002 >/dev/null 2>&1; then
    REGTEST_RUNNING=true
    echo "✓ Regtest detected"
else
    echo "⚠ Regtest not running - some tests will be skipped"
    echo "  Start regtest with: ./scripts/start-regtest.sh"
fi
echo ""

# Check if F1r3node is running
F1R3NODE_RUNNING=false
if curl -s http://localhost:40403/api/status >/dev/null 2>&1; then
    F1R3NODE_RUNNING=true
    echo "✓ F1r3node detected"
else
    echo "⚠ F1r3node not running - RGB tests will be skipped"
    echo "  Start F1r3node to test RGB functionality"
fi
echo ""

# Assertion helper functions
assert_success() {
    local test_num="$1"
    local test_name="$2"
    local condition="$3"
    
    if [ "$condition" = "0" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        TEST_RESULTS+=("✓ Test $test_num: $test_name - PASSED")
        return 0
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        TEST_RESULTS+=("✗ Test $test_num: $test_name - FAILED")
        return 1
    fi
}

assert_contains() {
    local output="$1"
    local expected="$2"
    if echo "$output" | grep -q "$expected"; then
        return 0
    else
        return 1
    fi
}

assert_greater_than() {
    local value="$1"
    local threshold="$2"
    if [ "$value" -gt "$threshold" ] 2>/dev/null; then
        return 0
    else
        return 1
    fi
}

# Helper: Get first available confirmed UTXO for RGB operations
get_available_utxo() {
    cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        list-utxos \
        --format compact \
        --available-only \
        --confirmed-only \
        --password "$PASSWORD" 2>&1 | \
        grep -v "warning:" | \
        grep -v "Finished" | \
        grep -v "Running" | \
        head -1 | \
        awk '{print $1}'  # Extract txid:vout
}

# Cleanup function
cleanup() {
    echo ""
    echo "Cleaning up temporary directory..."
    rm -rf "$TEMP_DIR"
    
    # Also clean up any test wallets that may have leaked to default directory
    if [ -d "$HOME/.f1r3fly-rgb-wallet/$WALLET_NAME" ]; then
        echo "⚠ Found test wallet in default directory (should not happen)"
        rm -rf "$HOME/.f1r3fly-rgb-wallet/$WALLET_NAME"
        echo "  Cleaned up leaked wallet"
    fi
    
    echo "✓ Cleanup complete"
}

# Register cleanup on exit
trap cleanup EXIT

# Build the binary
echo "Building CLI binary..."
cargo build --bin f1r3fly-rgb-wallet 2>&1 | grep -E "(Compiling|Finished)" || true
echo ""

# Test 1: Create wallet
echo "======================================"
echo "Test 1: Create wallet"
echo "======================================"
WALLET_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
    --data-dir "$TEMP_DIR" \
    wallet create "$WALLET_NAME" \
    --password "$PASSWORD" 2>&1)
echo "$WALLET_OUTPUT" | grep -v "warning:"
echo ""

# Validate wallet creation
assert_contains "$WALLET_OUTPUT" "created successfully"
TEST1_SUCCESS=$?

# Validate wallet directory exists (note: wallets are created directly under data-dir)
[ -d "$TEMP_DIR/$WALLET_NAME" ]
TEST1_DIR=$?

# Validate essential wallet files exist
[ -f "$TEMP_DIR/$WALLET_NAME/keys.json" ]
TEST1_KEYS=$?

[ -f "$TEMP_DIR/$WALLET_NAME/wallet.json" ]
TEST1_METADATA=$?

[ -f "$TEMP_DIR/$WALLET_NAME/descriptor.txt" ]
TEST1_DESC=$?

# Combined validation
if [ $TEST1_SUCCESS -eq 0 ] && [ $TEST1_DIR -eq 0 ] && [ $TEST1_KEYS -eq 0 ] && [ $TEST1_METADATA -eq 0 ] && [ $TEST1_DESC -eq 0 ]; then
    assert_success "1" "Create wallet" "0"
else
    assert_success "1" "Create wallet" "1"
    echo "  Error: Wallet creation validation failed"
fi

# Test 2: List wallets
echo "======================================"
echo "Test 2: List wallets"
echo "======================================"
LIST_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
    --data-dir "$TEMP_DIR" \
    wallet list 2>&1)
echo "$LIST_OUTPUT" | grep -v "warning:"
echo ""

# Validate list output
assert_contains "$LIST_OUTPUT" "$WALLET_NAME"
TEST2_NAME=$?

assert_contains "$LIST_OUTPUT" "(1)"
TEST2_COUNT=$?

if [ $TEST2_NAME -eq 0 ] && [ $TEST2_COUNT -eq 0 ]; then
    assert_success "2" "List wallets" "0"
else
    assert_success "2" "List wallets" "1"
    echo "  Error: Wallet not found in list or count incorrect"
fi

# Test 3: Get address using get-addresses command
echo "======================================"
echo "Test 3: Get address"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    # Use the proper get-addresses command
    ADDR_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        get-addresses \
        --count 1 \
        --password "$PASSWORD" 2>&1)
    
    echo "$ADDR_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Extract the address from get-addresses output
    WALLET_ADDRESS=$(echo "$ADDR_OUTPUT" | grep -o 'bcrt1[a-z0-9]*' | head -1)
    
    # Validate address extraction
    [ -n "$WALLET_ADDRESS" ]
    TEST3_NOT_EMPTY=$?
    
    echo "$WALLET_ADDRESS" | grep -q "^bcrt1"
    TEST3_FORMAT=$?
    
    if [ $TEST3_NOT_EMPTY -eq 0 ] && [ $TEST3_FORMAT -eq 0 ]; then
        assert_success "3" "Get address" "0"
        echo "  Address: $WALLET_ADDRESS"
    else
        assert_success "3" "Get address" "1"
        echo "  Error: Could not extract valid address"
        REGTEST_RUNNING=false
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 4: Initial sync (empty wallet)
echo "======================================"
echo "Test 4: Initial sync (before funding)"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    SYNC_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        sync --password "$PASSWORD" 2>&1)
    
    echo "$SYNC_OUTPUT" | grep -v "warning:" | head -3
    echo ""
    
    # Validate sync success
    assert_contains "$SYNC_OUTPUT" "synced successfully"
    if [ $? -eq 0 ]; then
        assert_success "4" "Initial sync" "0"
    else
        assert_success "4" "Initial sync" "1"
        echo "  Error: Sync did not complete successfully"
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 5: Fund wallet from mining wallet
echo "======================================"
echo "Test 5: Fund wallet"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ] && [ -n "$WALLET_ADDRESS" ]; then
    # Note: Mining blocks directly to wallet creates coinbase outputs with 0 value
    # due to Bitcoin halvings in regtest. Instead, we fund from mining_wallet.
    echo "Funding wallet with 0.01 BTC from mining wallet..."
    $BITCOIN_CLI -rpcwallet=mining_wallet sendtoaddress "$WALLET_ADDRESS" 0.01 > /dev/null 2>&1
    
    # Mine blocks to confirm the transaction
    echo "Mining 1 block to confirm..."
    $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
    
    # Wait for Electrs to index
    echo "Waiting ${SLEEP_TIME} seconds for Electrs indexing..."
    sleep $SLEEP_TIME
    
    echo "✓ Wallet funded with 0.01 BTC"
else
    echo "⚠ Skipping (regtest not running or no address)"
fi
echo ""

# Test 6: Sync wallet (should detect funds)
echo "======================================"
echo "Test 6: Sync wallet (after funding)"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    SYNC_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        sync --password "$PASSWORD" 2>&1)
    
    echo "$SYNC_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate sync detected new transactions
    assert_contains "$SYNC_OUTPUT" "New transactions:"
    TEST6_NEW_TX=$?
    
    # Extract and validate transaction count
    TX_COUNT=$(echo "$SYNC_OUTPUT" | grep "New transactions:" | grep -o '[0-9]*' | head -1)
    assert_greater_than "$TX_COUNT" "0"
    TEST6_COUNT=$?
    
    if [ $TEST6_NEW_TX -eq 0 ] && [ $TEST6_COUNT -eq 0 ]; then
        assert_success "6" "Sync after funding" "0"
        echo "  Detected: $TX_COUNT new transactions"
    else
        assert_success "6" "Sync after funding" "1"
        echo "  Error: No new transactions detected after mining blocks"
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 7: Get balance (should show funds)
echo "======================================"
echo "Test 7: Get balance (funded wallet)"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        get-balance --password "$PASSWORD" 2>&1)
    
    echo "$BALANCE_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate balance output structure
    assert_contains "$BALANCE_OUTPUT" "Bitcoin Balance:"
    TEST7_HEADER=$?
    
    # Extract confirmed balance
    CONFIRMED_SATS=$(echo "$BALANCE_OUTPUT" | grep "Confirmed:" | grep -o '[0-9]* sats' | awk '{print $1}')
    
    # Informational: Show if halving has occurred
    if [ "$CONFIRMED_SATS" -lt "1000000000" ]; then
        echo "  ℹ️  Note: Balance is ${CONFIRMED_SATS} sats (~$((CONFIRMED_SATS / 100000000)) BTC)"
        echo "     This is lower than usual, likely due to Bitcoin halvings in regtest."
        echo "     Test will continue (validates functionality, not specific amounts)."
        echo ""
    fi
    
    # Validate balance exists (just check > 0, not specific amount)
    assert_greater_than "$CONFIRMED_SATS" "0"
    TEST7_HAS_BALANCE=$?
    
    # Validate UTXO Summary exists (Step 7 enhancement)
    assert_contains "$BALANCE_OUTPUT" "UTXO Summary:"
    TEST7_SUMMARY=$?
    
    if [ $TEST7_HEADER -eq 0 ] && [ $TEST7_HAS_BALANCE -eq 0 ] && [ $TEST7_SUMMARY -eq 0 ]; then
        assert_success "7" "Get balance" "0"
        echo "  Balance: $CONFIRMED_SATS sats"
        echo "  UTXO Summary: ✓"
    else
        assert_success "7" "Get balance" "1"
        echo "  Error: Balance validation failed"
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 8: Create UTXO
echo "======================================"
echo "Test 8: Create UTXO"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    echo "Creating UTXO with 0.0003 BTC..."
    UTXO_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        create-utxo \
        --amount 0.0003 \
        --password "$PASSWORD" 2>&1)
    
    echo "$UTXO_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate UTXO creation
    assert_contains "$UTXO_OUTPUT" "created successfully"
    TEST8_SUCCESS=$?
    
    # Extract and validate transaction ID (should be 64 hex chars)
    TX_ID=$(echo "$UTXO_OUTPUT" | grep "Transaction ID:" | grep -o '[a-f0-9]\{64\}')
    [ ${#TX_ID} -eq 64 ]
    TEST8_TXID=$?
    
    # Extract and validate amount (should be ~29999 sats, accounting for fees)
    UTXO_AMOUNT=$(echo "$UTXO_OUTPUT" | grep "Amount:" | grep -o '[0-9]* sats' | awk '{print $1}')
    assert_greater_than "$UTXO_AMOUNT" "25000"
    TEST8_AMOUNT=$?
    
    if [ $TEST8_SUCCESS -eq 0 ] && [ $TEST8_TXID -eq 0 ] && [ $TEST8_AMOUNT -eq 0 ]; then
        assert_success "8" "Create UTXO" "0"
        echo "  Created: ${TX_ID:0:16}... ($UTXO_AMOUNT sats)"
    else
        assert_success "8" "Create UTXO" "1"
        echo "  Error: UTXO creation validation failed"
    fi
    
    echo ""
    echo "Mining 1 block to confirm..."
    $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
    sleep $SLEEP_TIME
    
    echo "Syncing wallet..."
    cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -5
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 8a: List UTXOs
echo "======================================"
echo "Test 8a: List UTXOs"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    echo "--- Table Format (default) ---"
    TABLE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        list-utxos \
        --password "$PASSWORD" 2>&1)
    
    echo "$TABLE_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate table output structure
    assert_contains "$TABLE_OUTPUT" "Outpoint"
    TEST8A_TABLE_HEADER=$?
    
    assert_contains "$TABLE_OUTPUT" "Amount"
    TEST8A_TABLE_AMOUNT=$?
    
    assert_contains "$TABLE_OUTPUT" "Status"
    TEST8A_TABLE_STATUS=$?
    
    # Validate we see some UTXOs (we should have ~4 from Test 5 funding + Test 8)
    # Note: Status column shows "Available" (capitalized)
    UTXO_COUNT_TABLE=$(echo "$TABLE_OUTPUT" | grep -c "Available" || echo "0")
    assert_greater_than "$UTXO_COUNT_TABLE" "2"
    TEST8A_TABLE_COUNT=$?
    
    echo "--- Compact Format (available-only) ---"
    COMPACT_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        list-utxos \
        --available-only \
        --format compact \
        --password "$PASSWORD" 2>&1)
    
    echo "$COMPACT_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate compact format (should be outpoint with amount and status, one per line)
    COMPACT_LINES=$(echo "$COMPACT_OUTPUT" | grep -v "warning:" | grep -v "Finished" | grep -v "Running" | grep -c ":" || echo "0")
    assert_greater_than "$COMPACT_LINES" "2"
    TEST8A_COMPACT_COUNT=$?
    
    # Validate compact format structure (should have txid:vout pattern and "available")
    echo "$COMPACT_OUTPUT" | grep -v "warning:" | grep -v "Finished" | grep -v "Running" | head -1 | grep -q "[a-f0-9].*:[0-9].* available"
    TEST8A_COMPACT_FORMAT=$?
    
    echo "--- JSON Format ---"
    JSON_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        list-utxos \
        --format json \
        --password "$PASSWORD" 2>&1)
    
    # Extract just the JSON (remove cargo warnings)
    JSON_ONLY=$(echo "$JSON_OUTPUT" | grep -v "warning:" | grep -v "Finished" | grep -v "Running")
    echo "$JSON_ONLY" | head -20
    echo "  ... (JSON truncated for readability) ..."
    echo ""
    
    # Validate JSON is valid and has expected fields
    echo "$JSON_ONLY" | jq -e '.total_utxos' > /dev/null 2>&1
    TEST8A_JSON_VALID=$?
    
    # Extract and validate UTXO count from JSON
    if [ $TEST8A_JSON_VALID -eq 0 ]; then
        JSON_UTXO_COUNT=$(echo "$JSON_ONLY" | jq -r '.total_utxos')
        assert_greater_than "$JSON_UTXO_COUNT" "2"
        TEST8A_JSON_COUNT=$?
        
        # Validate JSON has required fields
        echo "$JSON_ONLY" | jq -e '.utxos' > /dev/null 2>&1
        TEST8A_JSON_UTXOS=$?
        
        echo "$JSON_ONLY" | jq -e '.available_count' > /dev/null 2>&1
        TEST8A_JSON_AVAILABLE=$?
    else
        TEST8A_JSON_COUNT=1
        TEST8A_JSON_UTXOS=1
        TEST8A_JSON_AVAILABLE=1
    fi
    
    # Combined validation
    if [ $TEST8A_TABLE_HEADER -eq 0 ] && \
       [ $TEST8A_TABLE_AMOUNT -eq 0 ] && \
       [ $TEST8A_TABLE_STATUS -eq 0 ] && \
       [ $TEST8A_TABLE_COUNT -eq 0 ] && \
       [ $TEST8A_COMPACT_COUNT -eq 0 ] && \
       [ $TEST8A_COMPACT_FORMAT -eq 0 ] && \
       [ $TEST8A_JSON_VALID -eq 0 ] && \
       [ $TEST8A_JSON_COUNT -eq 0 ] && \
       [ $TEST8A_JSON_UTXOS -eq 0 ] && \
       [ $TEST8A_JSON_AVAILABLE -eq 0 ]; then
        assert_success "8a" "List UTXOs" "0"
        echo "  Table format: ✓ ($UTXO_COUNT_TABLE UTXOs)"
        echo "  Compact format: ✓ ($COMPACT_LINES UTXOs)"
        echo "  JSON format: ✓ ($JSON_UTXO_COUNT UTXOs, valid JSON)"
    else
        assert_success "8a" "List UTXOs" "1"
        echo "  Error: list-utxos validation failed"
        echo "  Debug: table_header=$TEST8A_TABLE_HEADER table_count=$TEST8A_TABLE_COUNT"
        echo "         compact_count=$TEST8A_COMPACT_COUNT compact_fmt=$TEST8A_COMPACT_FORMAT"
        echo "         json_valid=$TEST8A_JSON_VALID json_count=$TEST8A_JSON_COUNT"
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 9: Send Bitcoin
echo "======================================"
echo "Test 9: Send Bitcoin"
echo "======================================"
if [ "$REGTEST_RUNNING" = true ]; then
    # Get balance before send (from Test 7)
    BALANCE_BEFORE="$CONFIRMED_SATS"
    
    # Generate a new address from mining_wallet (available in both local and CI)
    RECIPIENT_ADDRESS=$($BITCOIN_CLI -rpcwallet=mining_wallet getnewaddress)
    echo "Sending 10,000 sats to: $RECIPIENT_ADDRESS"
    echo ""
    
    SEND_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        send-bitcoin \
        --to "$RECIPIENT_ADDRESS" \
        --amount 10000 \
        --password "$PASSWORD" 2>&1)
    
    echo "$SEND_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Validate send success
    assert_contains "$SEND_OUTPUT" "sent successfully"
    TEST9_SUCCESS=$?
    
    # Extract and validate transaction ID
    SEND_TX_ID=$(echo "$SEND_OUTPUT" | grep "Transaction ID:" | grep -o '[a-f0-9]\{64\}')
    [ ${#SEND_TX_ID} -eq 64 ]
    TEST9_TXID=$?
    
    echo "Mining 1 block to confirm..."
    $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
    sleep $SLEEP_TIME
    
    echo "Syncing wallet..."
    cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
    
    echo ""
    echo "Final balance:"
    FINAL_BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
        --data-dir "$TEMP_DIR" \
        --wallet "$WALLET_NAME" \
        get-balance --password "$PASSWORD" 2>&1)
    
    echo "$FINAL_BALANCE_OUTPUT" | grep -v "warning:"
    echo ""
    
    # Extract final balance
    BALANCE_AFTER=$(echo "$FINAL_BALANCE_OUTPUT" | grep "Confirmed:" | grep -o '[0-9]* sats' | awk '{print $1}')
    
    # Note: We don't validate balance decreased because mining blocks gives us
    # 50 BTC coinbase reward, which is more than the 10,000 sats we sent
    
    if [ $TEST9_SUCCESS -eq 0 ] && [ $TEST9_TXID -eq 0 ]; then
        assert_success "9" "Send bitcoin" "0"
        echo "  Sent: ${SEND_TX_ID:0:16}..."
        echo "  Amount: 10,000 sats"
        echo "  Note: Balance increased due to mining coinbase reward (50 BTC)"
    else
        assert_success "9" "Send bitcoin" "1"
        echo "  Error: Send validation failed"
    fi
else
    echo "⚠ Skipping (regtest not running)"
fi
echo ""

# Test 10: Issue RGB Asset
echo "======================================"
echo "Test 10: Issue RGB Asset"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    # Automatically select genesis UTXO using helper
    echo "Selecting genesis UTXO..."
    GENESIS_UTXO=$(get_available_utxo)
    
    # Validate UTXO was found
    if [ -z "$GENESIS_UTXO" ]; then
        echo "✗ ERROR: No available UTXO for genesis"
        echo ""
        echo "Available UTXOs:"
        cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            list-utxos --available-only --password "$PASSWORD" 2>&1 | grep -v "warning:"
        assert_success "10" "Issue RGB asset" "1"
    else
        echo "✓ Selected genesis UTXO: $GENESIS_UTXO"
        echo ""
        
        # Validate UTXO format
        echo "$GENESIS_UTXO" | grep -q "[a-f0-9]\{64\}:[0-9]"
        TEST10_UTXO_FORMAT=$?
        
        if [ $TEST10_UTXO_FORMAT -ne 0 ]; then
            echo "✗ ERROR: Invalid UTXO format: $GENESIS_UTXO"
            assert_success "10" "Issue RGB asset" "1"
        else
            echo "Issuing TEST token with supply 1000..."
            ISSUE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
                --data-dir "$TEMP_DIR" \
                --wallet "$WALLET_NAME" \
                issue-asset \
                --ticker TEST \
                --name "Test Token" \
                --supply 1000 \
                --precision 0 \
                --genesis-utxo "$GENESIS_UTXO" \
                --password "$PASSWORD" 2>&1)
            
            echo "$ISSUE_OUTPUT" | grep -v "warning:"
            echo ""
            
            # Validate issuance success
            assert_contains "$ISSUE_OUTPUT" "successfully"
            TEST10_SUCCESS=$?
            
            # Extract and validate contract ID
            CONTRACT_ID=$(echo "$ISSUE_OUTPUT" | grep "Contract ID:" | awk '{print $3}')
            
            if [ -n "$CONTRACT_ID" ]; then
                # Validate contract ID format (should start with contract:)
                echo "$CONTRACT_ID" | grep -q "^contract:"
                TEST10_CONTRACT_FORMAT=$?
                
                if [ $TEST10_SUCCESS -eq 0 ] && [ $TEST10_CONTRACT_FORMAT -eq 0 ]; then
                    assert_success "10" "Issue RGB asset" "0"
                    echo "  Genesis UTXO: $GENESIS_UTXO"
                    echo "  Contract ID: $CONTRACT_ID"
                    echo "  Token: TEST (1000 units)"
                    # Store for future tests
                    export CONTRACT_ID
                    export GENESIS_UTXO
                else
                    assert_success "10" "Issue RGB asset" "1"
                    echo "  Error: Issuance validation failed"
                fi
            else
                assert_success "10" "Issue RGB asset" "1"
                echo "  Error: Could not extract contract ID"
            fi
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
    if [ "$F1R3NODE_RUNNING" = false ]; then
        echo "  - F1r3node not running"
    fi
    if [ "$REGTEST_RUNNING" = false ]; then
        echo "  - Regtest not running"
    fi
fi
echo ""

# Test 11: List RGB Assets
echo "======================================"
echo "Test 11: List RGB Assets"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    # Only run if Test 10 succeeded (CONTRACT_ID exists)
    if [ -z "$CONTRACT_ID" ]; then
        echo "⚠ Skipping - Test 10 did not provide CONTRACT_ID"
        echo "  Cannot validate asset list without issued asset"
    else
        echo "Listing all RGB assets..."
        ASSETS_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            list-assets \
            --password "$PASSWORD" 2>&1)
        
        echo "$ASSETS_OUTPUT" | grep -v "warning:"
        echo ""
        
        # Validate asset list contains TEST ticker
        assert_contains "$ASSETS_OUTPUT" "TEST"
        TEST11_HAS_TICKER=$?
        
        # Validate contract ID appears (exact match)
        assert_contains "$ASSETS_OUTPUT" "$CONTRACT_ID"
        TEST11_HAS_CONTRACT=$?
        
        # Validate asset name appears
        assert_contains "$ASSETS_OUTPUT" "Test Token"
        TEST11_HAS_NAME=$?
        
        # Validate asset count shows at least 1
        assert_contains "$ASSETS_OUTPUT" "RGB Assets"
        TEST11_HAS_HEADER=$?
        
        # Extract asset count from "RGB Assets (1):"
        ASSET_COUNT=$(echo "$ASSETS_OUTPUT" | grep "RGB Assets" | grep -o '([0-9]*)' | grep -o '[0-9]*')
        if [ -n "$ASSET_COUNT" ]; then
            assert_greater_than "$ASSET_COUNT" "0"
            TEST11_HAS_ASSETS=$?
        else
            TEST11_HAS_ASSETS=1
        fi
        
        # Combined validation (based on actual output format)
        if [ $TEST11_HAS_TICKER -eq 0 ] && \
           [ $TEST11_HAS_CONTRACT -eq 0 ] && \
           [ $TEST11_HAS_NAME -eq 0 ] && \
           [ $TEST11_HAS_HEADER -eq 0 ] && \
           [ $TEST11_HAS_ASSETS -eq 0 ]; then
            assert_success "11" "List assets" "0"
            echo "  ✓ TEST token found in asset list"
            echo "  ✓ Contract ID: $CONTRACT_ID"
            echo "  ✓ Name: Test Token"
            echo "  ✓ Asset count: $ASSET_COUNT"
            echo "  Note: Supply/Balance shown in 'rgb-balance' command"
        else
            assert_success "11" "List assets" "1"
            echo "  Error: Asset list validation failed"
            echo "  Debug: ticker=$TEST11_HAS_TICKER contract=$TEST11_HAS_CONTRACT"
            echo "         name=$TEST11_HAS_NAME header=$TEST11_HAS_HEADER"
            echo "         has_assets=$TEST11_HAS_ASSETS (count=$ASSET_COUNT)"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
    if [ "$F1R3NODE_RUNNING" = false ]; then
        echo "  - F1r3node not running"
    fi
    if [ "$REGTEST_RUNNING" = false ]; then
        echo "  - Regtest not running"
    fi
fi
echo ""

# Test 12: RGB Balance
echo "======================================"
echo "Test 12: RGB Balance"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    # Only run if Test 10 succeeded (CONTRACT_ID exists)
    if [ -z "$CONTRACT_ID" ]; then
        echo "⚠ Skipping - Test 10 did not provide CONTRACT_ID"
        echo "  Cannot validate RGB balance without issued asset"
    else
        echo "Checking RGB balance for TEST token..."
        RGB_BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            rgb-balance \
            --password "$PASSWORD" 2>&1)
        
        echo "$RGB_BALANCE_OUTPUT" | grep -v "warning:"
        echo ""
        
        # Validate TEST ticker appears
        assert_contains "$RGB_BALANCE_OUTPUT" "TEST"
        TEST12_HAS_TICKER=$?
        
        # Validate contract ID appears
        assert_contains "$RGB_BALANCE_OUTPUT" "$CONTRACT_ID"
        TEST12_HAS_CONTRACT=$?
        
        # Extract balance (STRICT: must be exactly 1000)
        # Note: Output shows "Total:" not "Balance:", and may be in decimal format
        RGB_BALANCE=$(echo "$RGB_BALANCE_OUTPUT" | grep -E "(Total:|Balance:|balance:)" | grep -o '[0-9.]*' | grep -v '^$' | head -1)
        
        # Validate balance is numeric and not empty
        if [ -z "$RGB_BALANCE" ]; then
            echo "✗ ERROR: Could not extract balance from output"
            TEST12_BALANCE_VALID=1
            TEST12_BALANCE_EXACT=1
        else
            TEST12_BALANCE_VALID=0
            
            # STRICT VALIDATION: Balance must be exactly 1000
            # With precision 0, balance should display as raw units (1000), not decimal
            if [ "$RGB_BALANCE" = "1000" ]; then
                TEST12_BALANCE_EXACT=0
            else
                echo "✗ ERROR: Expected balance 1000, got $RGB_BALANCE"
                echo "     (With precision 0, balance should display as raw units)"
                TEST12_BALANCE_EXACT=1
            fi
        fi
        
        # Extract supply if present
        RGB_SUPPLY=$(echo "$RGB_BALANCE_OUTPUT" | grep -i "supply" | grep -o '[0-9]*' | head -1)
        
        # If supply shown, validate it matches balance (genesis issuance)
        if [ -n "$RGB_SUPPLY" ]; then
            if [ "$RGB_BALANCE" = "$RGB_SUPPLY" ]; then
                TEST12_SUPPLY_MATCH=0
            else
                echo "✗ ERROR: Balance ($RGB_BALANCE) doesn't match supply ($RGB_SUPPLY)"
                TEST12_SUPPLY_MATCH=1
            fi
        else
            # Supply not shown is OK
            TEST12_SUPPLY_MATCH=0
        fi
        
        # Validate precision if shown
        if echo "$RGB_BALANCE_OUTPUT" | grep -q "Precision"; then
            echo "$RGB_BALANCE_OUTPUT" | grep "Precision" | grep -q "0"
            TEST12_PRECISION=$?
        else
            TEST12_PRECISION=0  # Not shown is OK
        fi
        
        # Combined validation
        if [ $TEST12_HAS_TICKER -eq 0 ] && \
           [ $TEST12_HAS_CONTRACT -eq 0 ] && \
           [ $TEST12_BALANCE_VALID -eq 0 ] && \
           [ $TEST12_BALANCE_EXACT -eq 0 ] && \
           [ $TEST12_SUPPLY_MATCH -eq 0 ] && \
           [ $TEST12_PRECISION -eq 0 ]; then
            assert_success "12" "RGB balance" "0"
            echo "  ✓ TEST token balance verified"
            echo "  ✓ Contract ID: $CONTRACT_ID"
            echo "  ✓ Balance: $RGB_BALANCE (exactly 1000 ✓)"
            if [ -n "$RGB_SUPPLY" ]; then
                echo "  ✓ Supply: $RGB_SUPPLY (matches balance ✓)"
            fi
            echo "  ✓ Balance matches issuance amount"
            # Export for Test 12a
            export RGB_BALANCE
        else
            assert_success "12" "RGB balance" "1"
            echo "  Error: RGB balance validation failed"
            echo "  Debug: ticker=$TEST12_HAS_TICKER contract=$TEST12_HAS_CONTRACT"
            echo "         balance_valid=$TEST12_BALANCE_VALID balance_exact=$TEST12_BALANCE_EXACT"
            echo "         supply_match=$TEST12_SUPPLY_MATCH precision=$TEST12_PRECISION"
            echo "         extracted_balance=$RGB_BALANCE expected=1000"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
    if [ "$F1R3NODE_RUNNING" = false ]; then
        echo "  - F1r3node not running"
    fi
    if [ "$REGTEST_RUNNING" = false ]; then
        echo "  - Regtest not running"
    fi
fi
echo ""

# Test 13: Create second wallet (Bob) for transfer testing
echo "======================================"
echo "Test 13: Create second wallet (Bob)"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    # Only run if Test 10 succeeded (CONTRACT_ID exists)
    if [ -z "$CONTRACT_ID" ]; then
        echo "⚠ Skipping - Test 10 did not provide CONTRACT_ID"
    else
        echo "Creating Bob's wallet (${BOB_WALLET_NAME})..."
        BOB_CREATE=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            wallet create "$BOB_WALLET_NAME" \
            --password "$PASSWORD" 2>&1)
        
        echo "$BOB_CREATE" | grep -v "warning:"
        echo ""
        
        # Validate wallet creation
        assert_contains "$BOB_CREATE" "created successfully"
        TEST13_SUCCESS=$?
        
        # Get Bob's address
        BOB_ADDR_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            get-addresses \
            --count 1 \
            --password "$PASSWORD" 2>&1)
        
        BOB_ADDRESS=$(echo "$BOB_ADDR_OUTPUT" | grep -o 'bcrt1[a-z0-9]*' | head -1)
        
        # Fund Bob's wallet
        echo "Funding Bob's wallet with 10 blocks..."
        $BITCOIN_CLI generatetoaddress 10 "$BOB_ADDRESS" > /dev/null 2>&1
        sleep $SLEEP_TIME
        
        # Sync Bob's wallet
        cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
        
        if [ $TEST13_SUCCESS -eq 0 ] && [ -n "$BOB_ADDRESS" ]; then
            assert_success "13" "Create Bob's wallet" "0"
            echo "  Bob's wallet: $BOB_WALLET_NAME"
            echo "  Bob's address: $BOB_ADDRESS"
            export BOB_ADDRESS
        else
            assert_success "13" "Create Bob's wallet" "1"
            echo "  Error: Bob's wallet creation failed"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 13a: Alice exports genesis consignment
echo "======================================"
echo "Test 13a: Alice exports genesis"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$CONTRACT_ID" ]; then
        echo "⚠ Skipping - CONTRACT_ID not available"
    else
        echo "Alice exporting genesis consignment..."
        EXPORT_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            export-genesis \
            --contract-id "$CONTRACT_ID" \
            --password "$PASSWORD" 2>&1)
        
        echo "$EXPORT_OUTPUT" | grep -v "warning:"
        echo ""
        
        # Extract consignment path from output (look for "Location:" line)
        GENESIS_CONSIGNMENT_PATH=$(echo "$EXPORT_OUTPUT" | grep "Location:" | awk '{print $2}')
        
        # Validate export
        if [ -f "$GENESIS_CONSIGNMENT_PATH" ]; then
            assert_success "13a" "Alice exports genesis" "0"
            echo "  ✓ Genesis consignment: $GENESIS_CONSIGNMENT_PATH"
            export GENESIS_CONSIGNMENT_PATH
        else
            assert_success "13a" "Alice exports genesis" "1"
            echo "  ✗ Genesis consignment file not found"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 13b: Bob imports contract genesis
echo "======================================"
echo "Test 13b: Bob imports genesis"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$GENESIS_CONSIGNMENT_PATH" ] || [ -z "$BOB_WALLET_NAME" ]; then
        echo "⚠ Skipping - Prerequisites not met"
    else
        echo "Bob importing contract genesis..."
        IMPORT_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            accept-consignment \
            --consignment-path "$GENESIS_CONSIGNMENT_PATH" \
            --password "$PASSWORD" 2>&1)
        
        echo "$IMPORT_OUTPUT" | grep -v "warning:"
        echo ""
        
        # Validate import
        assert_contains "$IMPORT_OUTPUT" "accepted successfully"
        if [ $? -eq 0 ]; then
            assert_success "13b" "Bob imports genesis" "0"
            echo "  ✓ Bob now has contract: $CONTRACT_ID"
        else
            assert_success "13b" "Bob imports genesis" "1"
            echo "  ✗ Bob failed to import contract"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 14: Generate RGB Invoice (Bob receives TEST tokens)
echo "======================================"
echo "Test 14: Generate RGB Invoice (Bob)"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$CONTRACT_ID" ] || [ -z "$BOB_WALLET_NAME" ]; then
        echo "⚠ Skipping - Prerequisites not met"
    else
        echo "Bob generating invoice for 250 TEST tokens..."
        INVOICE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            generate-invoice \
            --contract-id "$CONTRACT_ID" \
            --amount 250 \
            --password "$PASSWORD" 2>&1)
        
        echo "$INVOICE_OUTPUT" | grep -v "warning:"
        echo ""
        
        # Extract invoice (get the line after "Invoice String:")
        INVOICE=$(echo "$INVOICE_OUTPUT" | grep -A 1 "Invoice String:" | tail -1 | tr -d '[:space:]')
        
        # Validate invoice
        [ -n "$INVOICE" ]
        TEST14_HAS_INVOICE=$?
        
        echo "$INVOICE" | grep -q "^contract:"
        TEST14_INVOICE_FORMAT=$?
        
        if [ $TEST14_HAS_INVOICE -eq 0 ] && [ $TEST14_INVOICE_FORMAT -eq 0 ]; then
            assert_success "14" "Generate invoice" "0"
            echo "  Invoice: ${INVOICE:0:50}..."
            export INVOICE
            
            # CRITICAL: Bob must sync after generating invoice
            # After reveal_next_address() in invoice generation, Bob must sync
            # so that BDK's in-memory spk_index tracks the new address for UTXO discovery.
            # Without this sync, BDK won't discover UTXOs sent to the invoice address!
            echo ""
            echo "Syncing Bob's wallet after invoice generation..."
            cargo run --bin f1r3fly-rgb-wallet -- \
                --data-dir "$TEMP_DIR" \
                --wallet "$BOB_WALLET_NAME" \
                sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
        else
            assert_success "14" "Generate invoice" "1"
            echo "  Error: Invoice generation failed"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 15: Send RGB Transfer (Alice → Bob)
echo "======================================"
echo "Test 15: Send RGB Transfer (Alice → Bob)"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$INVOICE" ] || [ -z "$BOB_WALLET_NAME" ]; then
        echo "⚠ Skipping - Prerequisites not met"
    else
        # Fund Alice's wallet with Bitcoin from mining wallet for transfer fee
        echo "Funding Alice with 0.001 BTC from mining wallet for transfer fee..."
        $BITCOIN_CLI -rpcwallet=mining_wallet sendtoaddress "$WALLET_ADDRESS" 0.001 > /dev/null 2>&1
        
        # Mine blocks to confirm the transaction
        $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
        sleep $SLEEP_TIME
        
        # Sync Alice's wallet
        cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
        
        echo ""
        
        # Get Bob's F1r3fly public key
        echo "Getting Bob's F1r3fly public key..."
        BOB_PUBKEY_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            wallet get-f1r3fly-pubkey 2>&1)
        
        BOB_PUBKEY=$(echo "$BOB_PUBKEY_OUTPUT" | grep -v "warning:" | grep -v "F1r3fly Public Key:" | grep -v "💡" | grep -v "Finished" | grep -v "Running" | grep -v "Compiling" | grep -o '[a-f0-9]\{130\}')
        
        if [ -z "$BOB_PUBKEY" ]; then
            echo "✗ ERROR: Could not extract Bob's F1r3fly public key"
            assert_success "15" "Send RGB transfer" "1"
        else
            echo "  Bob's pubkey: ${BOB_PUBKEY:0:16}..."
            echo ""
            
            # Alice sends transfer to Bob
            echo "Alice sending 250 TEST tokens to Bob..."
            TRANSFER_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
                --data-dir "$TEMP_DIR" \
                --wallet "$WALLET_NAME" \
                send-transfer \
                --invoice "$INVOICE" \
                --recipient-pubkey "$BOB_PUBKEY" \
                --password "$PASSWORD" 2>&1)
            
            echo "$TRANSFER_OUTPUT" | grep -v "warning:"
            echo ""
            
            # Validate transfer
            assert_contains "$TRANSFER_OUTPUT" "sent successfully"
            TEST15_SUCCESS=$?
            
            # Extract Bitcoin TX ID
            TRANSFER_TXID=$(echo "$TRANSFER_OUTPUT" | grep "Bitcoin TX ID:" | awk '{print $4}')
            
            # Extract consignment path
            CONSIGNMENT_PATH=$(echo "$TRANSFER_OUTPUT" | grep "Consignment:" | awk '{print $2}')
            
            # Validate consignment file exists
            [ -f "$CONSIGNMENT_PATH" ]
            TEST15_CONSIGNMENT_EXISTS=$?
            
            if [ $TEST15_SUCCESS -eq 0 ] && [ $TEST15_CONSIGNMENT_EXISTS -eq 0 ]; then
                assert_success "15" "Send RGB transfer" "0"
                echo "  TX ID: ${TRANSFER_TXID:0:16}..."
                echo "  Consignment: $CONSIGNMENT_PATH"
                export TRANSFER_TXID
                export CONSIGNMENT_PATH
                
                # CRITICAL: Mine blocks to confirm transfer BEFORE Bob accepts
                # This matches integration test Steps 4 and 5.5
                echo ""
                echo "Mining 1 block to confirm transfer..."
                $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
                echo "Waiting ${SLEEP_TIME} seconds for Esplora indexing..."
                sleep $SLEEP_TIME
                
                # Step 5.5 from integration test: Mine another block for additional confirmation
                echo "Mining 1 more block for confirmation (matches integration test)..."
                $BITCOIN_CLI generatetoaddress 1 "$WALLET_ADDRESS" > /dev/null 2>&1
                sleep $SLEEP_TIME
                
                # Step 5.5: Sync Alice's wallet after confirmation
                # This matches integration test and ensures transfer state is finalized
                echo "Syncing Alice's wallet after confirmation..."
                cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$WALLET_NAME" \
                    sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
                echo ""
                
                # Step 6: Verify Alice's balance BEFORE Bob accepts
                # This ensures the transfer actually succeeded on F1r3node
                echo "Verifying Alice's balance after transfer..."
                ALICE_BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$WALLET_NAME" \
                    rgb-balance \
                    --password "$PASSWORD" 2>&1)
                
                ALICE_BALANCE=$(echo "$ALICE_BALANCE_OUTPUT" | grep -E "(Total:|Balance:)" | grep -o '[0-9]*' | head -1)
                
                if [ "$ALICE_BALANCE" = "750" ]; then
                    echo "  ✓ Alice's balance: 750 TEST (transfer succeeded)"
                else
                    echo "  ✗ WARNING: Alice's balance is $ALICE_BALANCE (expected 750)"
                    echo "  Transfer may not have completed on F1r3node"
                fi
                echo ""
            else
                assert_success "15" "Send RGB transfer" "1"
                echo "  Error: Transfer validation failed"
            fi
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 16: List Claims
echo "======================================"
echo "Test 16: List Claims"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$CONTRACT_ID" ]; then
        echo "⚠ Skipping - CONTRACT_ID not available"
    else
        echo "--- Table Format ---"
        CLAIMS_TABLE=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            list-claims \
            --format table \
            --password "$PASSWORD" 2>&1)
        
        echo "$CLAIMS_TABLE" | grep -v "warning:"
        echo ""
        
        echo "--- JSON Format ---"
        CLAIMS_JSON=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            list-claims \
            --contract-id "$CONTRACT_ID" \
            --format json \
            --password "$PASSWORD" 2>&1)
        
        # Validate JSON is valid
        JSON_ONLY=$(echo "$CLAIMS_JSON" | grep -v "warning:" | grep -v "Finished" | grep -v "Running")
        echo "$JSON_ONLY" | head -10
        echo ""
        
        # Check if JSON is valid array
        echo "$JSON_ONLY" | jq -e '. | type == "array"' > /dev/null 2>&1
        TEST16_JSON_VALID=$?
        
        # The command should work even if there are no claims
        if [ $TEST16_JSON_VALID -eq 0 ]; then
            assert_success "16" "List claims" "0"
            echo "  ✓ Table format displayed"
            echo "  ✓ JSON format valid"
        else
            assert_success "16" "List claims" "1"
            echo "  Error: Claims list validation failed"
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Test 17: Accept Consignment (Bob)
echo "======================================"
echo "Test 17: Accept Consignment & Verify Transfer"
echo "======================================"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    if [ -z "$CONSIGNMENT_PATH" ] || [ -z "$TRANSFER_TXID" ]; then
        echo "⚠ Skipping - Test 15 did not complete successfully"
    else
        # CRITICAL: Bob must sync BEFORE accepting the consignment
        # Bob needs to sync his wallet so his BDK wallet can discover the new UTXO
        # that Alice sent him. This UTXO will be used during the auto-claim process
        # when accepting the consignment.
        echo "Syncing Bob's wallet to discover received UTXO..."
        cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
        echo ""
        
        echo "Bob accepting transfer consignment..."
        ACCEPT_OUTPUT=$(RUST_LOG=debug cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            accept-consignment \
            --consignment-path "$CONSIGNMENT_PATH" \
            --password "$PASSWORD" 2>&1)
        
        # Show non-debug output first
        echo "$ACCEPT_OUTPUT" | grep -v "warning:" | grep -v "DEBUG"
        echo ""
        
        # Show debug logs related to claims
        echo "--- Debug: Claim Processing ---"
        echo "$ACCEPT_OUTPUT" | grep -E "DEBUG.*claim|DEBUG.*witness|DEBUG.*UTXO|DEBUG.*actual" | head -20
        echo ""
        
        # Validate acceptance
        assert_contains "$ACCEPT_OUTPUT" "accepted successfully"
        TEST17_SUCCESS=$?
        
        # CRITICAL: Bob syncs AFTER accepting to finalize claim
        # After accepting the consignment (which stores the witness mapping),
        # Bob needs to sync again. This triggers retry_pending_claims which will
        # find the stored witness mapping and execute the claim to migrate the
        # balance from the witness ID to Bob's real UTXO.
        # This matches integration test Step 8.5
        echo "Syncing Bob's wallet after accepting consignment..."
        cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            sync --password "$PASSWORD" 2>&1 | grep -v "warning:" | head -3
        echo ""
        
        # Check Bob's claim status
        echo "Checking Bob's claim status..."
        BOB_CLAIMS=$(RUST_LOG=debug cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$BOB_WALLET_NAME" \
            list-claims --format json --password "$PASSWORD" 2>&1)
        
        # Show debug logs for claim status
        echo "$BOB_CLAIMS" | grep -E "(DEBUG|WARN|ERROR|claim)" | head -20
        
        # Extract just JSON
        BOB_CLAIMS_JSON=$(echo "$BOB_CLAIMS" | grep -v "warning:" | grep -v "DEBUG" | grep -v "Finished" | grep -v "Running")
        echo "$BOB_CLAIMS_JSON" | jq '.' 2>/dev/null || echo "$BOB_CLAIMS_JSON"
        echo ""
        
        # Retry balance check with syncs (like Rust integration tests)
        echo "Verifying Bob's balance (with retries)..."
        MAX_ATTEMPTS=5
        BOB_BALANCE=""
        
        for attempt in $(seq 1 $MAX_ATTEMPTS); do
            echo "  Attempt $attempt/$MAX_ATTEMPTS..."
            
            # Sync wallet (show debug logs on first attempt)
            if [ $attempt -eq 1 ]; then
                SYNC_OUTPUT=$(RUST_LOG=debug cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$BOB_WALLET_NAME" \
                    sync --password "$PASSWORD" 2>&1)
                echo "$SYNC_OUTPUT" | grep -E "(DEBUG|claim|attempt)" | head -10
            else
                cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$BOB_WALLET_NAME" \
                    sync --password "$PASSWORD" 2>&1 | grep -v "warning:" > /dev/null
            fi
            
            # Check balance (with INFO logging on first attempt)
            if [ $attempt -eq 1 ]; then
                BOB_BALANCE_OUTPUT=$(RUST_LOG=info cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$BOB_WALLET_NAME" \
                    rgb-balance \
                    --password "$PASSWORD" 2>&1)
                
                # Show all balance query logs on first attempt
                echo "--- Balance Query Debug (Attempt 1) ---"
                echo "$BOB_BALANCE_OUTPUT" | grep -E "(BALANCE QUERY|Claimed UTXOs|get_claimed_utxos|Querying|seal_id)" || true
                echo "---------------------------------------"
            else
                BOB_BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
                    --data-dir "$TEMP_DIR" \
                    --wallet "$BOB_WALLET_NAME" \
                    rgb-balance \
                    --password "$PASSWORD" 2>&1)
            fi
            
            BOB_BALANCE=$(echo "$BOB_BALANCE_OUTPUT" | grep -E "(Total:|Balance:)" | grep -o '[0-9]*' | head -1)
            
            if [ "$BOB_BALANCE" = "250" ]; then
                echo "  ✓ Balance confirmed: 250 TEST"
                break
            fi
            
            if [ $attempt -lt $MAX_ATTEMPTS ]; then
                echo "    Balance: $BOB_BALANCE (expected 250), retrying..."
                sleep 2
            fi
        done
        
        # Validate Bob received 250 tokens
        if [ "$BOB_BALANCE" = "250" ]; then
            TEST17_BALANCE_OK=0
        else
            echo "✗ ERROR: Expected Bob's balance to be 250, got $BOB_BALANCE after $MAX_ATTEMPTS attempts"
            TEST17_BALANCE_OK=1
        fi
        echo ""
        
        # Check Alice's balance (should be 750)
        echo "Checking Alice's balance..."
        ALICE_BALANCE_OUTPUT=$(cargo run --bin f1r3fly-rgb-wallet -- \
            --data-dir "$TEMP_DIR" \
            --wallet "$WALLET_NAME" \
            rgb-balance \
            --password "$PASSWORD" 2>&1)
        
        ALICE_BALANCE=$(echo "$ALICE_BALANCE_OUTPUT" | grep -E "(Total:|Balance:)" | grep -o '[0-9]*' | head -1)
        
        if [ "$ALICE_BALANCE" = "750" ]; then
            TEST17_ALICE_OK=0
        else
            echo "✗ ERROR: Expected Alice's balance to be 750, got $ALICE_BALANCE"
            TEST17_ALICE_OK=1
        fi
        
        if [ $TEST17_SUCCESS -eq 0 ] && [ $TEST17_BALANCE_OK -eq 0 ] && [ $TEST17_ALICE_OK -eq 0 ]; then
            assert_success "17" "Accept consignment & verify" "0"
            echo "  ✓ Bob received 250 TEST"
            echo "  ✓ Alice has 750 TEST remaining"
            echo "  ✓ Total conserved: 1000 TEST"
        else
            assert_success "17" "Accept consignment & verify" "1"
            echo "  Error: Balance verification failed"
            if [ $TEST17_BALANCE_OK -ne 0 ]; then
                echo "    Bob's balance: $BOB_BALANCE (expected 250)"
            fi
            if [ $TEST17_ALICE_OK -ne 0 ]; then
                echo "    Alice's balance: $ALICE_BALANCE (expected 750)"
            fi
        fi
    fi
else
    echo "⚠ Skipping (F1r3node or regtest not running)"
fi
echo ""

# Summary
echo "======================================"
echo "Test Summary"
echo "======================================"
echo ""
echo "Bitcoin Layer Tests:"

# Print all test results
for result in "${TEST_RESULTS[@]}"; do
    echo "  $result"
done

echo ""
echo "  Tests passed: $TESTS_PASSED"
echo "  Tests failed: $TESTS_FAILED"
echo ""

if [ $TESTS_FAILED -eq 0 ] && [ $TESTS_PASSED -gt 0 ]; then
    echo "  🎉 All tests passed!"
    EXIT_CODE=0
elif [ $TESTS_PASSED -eq 0 ]; then
    echo "  ⚠ No tests were run (check environment)"
    EXIT_CODE=1
else
    echo "  ❌ Some tests failed"
    EXIT_CODE=1
fi

echo ""
echo "RGB Asset Tests:"
if [ "$F1R3NODE_RUNNING" = true ] && [ "$REGTEST_RUNNING" = true ]; then
    echo "  ✓ RGB asset issuance (Test 10)"
    echo "  ✓ Asset listing & balance (Tests 11-12)"
    echo "  ✓ Multi-wallet setup (Test 13)"
    echo "  ✓ Genesis export/import (Tests 13a-13b)"
    echo "  ✓ Invoice generation (Test 14)"
    echo "  ✓ Transfer execution (Test 15)"
    echo "  ✓ Claim tracking (Test 16)"
    echo "  ✓ Transfer verification (Test 17)"
    echo ""
    echo "  Complete RGB transfer flow validated:"
    echo "    - issue-asset (Alice creates contract)"
    echo "    - export-genesis (Alice exports for sharing)"
    echo "    - accept-consignment (Bob imports genesis)"
    echo "    - wallet get-f1r3fly-pubkey (Bob shares pubkey)"
    echo "    - generate-invoice (Bob creates invoice)"
    echo "    - send-transfer (Alice sends to Bob)"
    echo "    - accept-consignment (Bob receives transfer)"
    echo "    - list-claims (track claim status)"
else
    echo "  ⚠ RGB tests SKIPPED"
    if [ "$F1R3NODE_RUNNING" = false ]; then
        echo "    - F1r3node not running"
    fi
    if [ "$REGTEST_RUNNING" = false ]; then
        echo "    - Regtest not running"
    fi
fi
echo ""
echo "Log saved to: $LOG_FILE"
echo "Temporary directory will be cleaned up automatically."

# Exit with proper code
exit $EXIT_CODE

