//! Bitcoin layer integration tests
//!
//! These tests require a running regtest environment.
//! Start with: ./scripts/start-regtest.sh

pub mod wallet_test;
pub mod network_sync_test;
pub mod balance_test;
pub mod utxo_test;
pub mod send_test;
pub mod manager_test;
pub mod cli_flow_test;

