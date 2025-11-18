//! F1r3fly-RGB wallet layer integration tests
//!
//! These tests focus on F1r3fly-RGB asset issuance and balance queries.
//!
//! Prerequisites:
//! - Running regtest environment: ./scripts/start-regtest.sh
//! - Running F1r3node with environment variables:
//!   - FIREFLY_GRPC_HOST (default: localhost)
//!   - FIREFLY_GRPC_PORT (default: 40401)
//!   - FIREFLY_HTTP_PORT (default: 40403)
//!   - FIREFLY_PRIVATE_KEY (required)
//!
//! Run tests:
//! ```bash
//! # All F1r3fly tests
//! cargo test --test f1r3fly_integration_tests
//!
//! # With output
//! cargo test --test f1r3fly_integration_tests -- --nocapture
//!
//! # Specific test
//! cargo test --test f1r3fly_integration_tests test_issue_single_asset -- --nocapture
//! ```
//!
//! Note: Tests will skip gracefully if F1r3node is not available.

mod common;
mod f1r3fly;
