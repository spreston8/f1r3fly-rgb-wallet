//! Bitcoin layer integration tests
//!
//! These tests require a running regtest environment.
//! Start with: ./scripts/start-regtest.sh
//!
//! Run tests:
//! ```bash
//! # All tests in parallel
//! cargo test --test bitcoin_integration_tests
//!
//! # Sequential (if needed)
//! cargo test --test bitcoin_integration_tests -- --test-threads=1
//!
//! # With output
//! cargo test --test bitcoin_integration_tests -- --nocapture
//! ```

mod bitcoin;
mod common;
