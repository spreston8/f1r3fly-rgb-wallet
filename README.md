# f1r3fly-rgb-wallet

CLI wallet for managing Bitcoin and F1r3fly-RGB assets with hierarchical deterministic key management.

## Features

- **Bitcoin Wallet**: BDK-based Bitcoin wallet with regtest/testnet/mainnet support
- **RGB Asset Management**: Issue and manage RGB assets on F1r3fly
- **Key Management**: BIP39 mnemonic with encrypted storage (AES-256-GCM + PBKDF2)
- **CLI Interface**: Commands for wallet operations, balance queries, and asset issuance
- **Test Environment**: Comprehensive integration tests with Bitcoin regtest

## Running Tests

### Prerequisites

Bitcoin regtest and F1r3node must be running for integration tests. See `.env.example` for required environment variables.

### Test Suites

```bash
# All tests (F1r3fly tests need sequential execution)
cargo test -- --test-threads=1

# Keys and storage tests (no prerequisites, can run in parallel)
cargo test --test keys_test
cargo test --test storage_test

# Bitcoin integration tests (requires regtest, must run sequentially)
cargo test --test bitcoin_integration_tests -- --test-threads=1

# F1r3fly-RGB tests (requires regtest + F1r3node, must run sequentially)
cargo test --test f1r3fly_integration_tests -- --test-threads=1

# List UTXOs integration tests (requires regtest + F1r3node, must run sequentially)
cargo test --test list_utxos_integration_test -- --test-threads=1 test_bitcoin_layer_list_utxos test_rgb_layer_seal_info test_manager_orchestration_with_filters test_multiple_rgb_assets test_data_structures_and_edge_cases

# Unit tests
cargo test --lib

# Specific test
cargo test test_create_wallet
```

**Note**: Bitcoin and F1r3fly-RGB integration tests must run with `--test-threads=1` due to shared blockchain state.

## CLI Usage

```bash
# Build
cargo build --release

# Create wallet
./target/release/f1r3fly-rgb-wallet wallet create my_wallet

# Get balance
./target/release/f1r3fly-rgb-wallet balance --wallet my_wallet --password <password>
```

See `./test_cli.sh` for complete CLI workflow examples.

