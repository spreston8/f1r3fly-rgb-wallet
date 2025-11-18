//! F1r3fly-RGB Wallet Integration Test Helpers
//!
//! Shared test infrastructure for F1r3fly-RGB wallet layer integration tests.
//! These tests focus on wallet-specific wrappers and state persistence.

use f1r3fly_rgb_wallet::manager::WalletManager;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

// Re-export common test utilities
pub use crate::common::TestBitcoinEnv;

// Test modules
mod asset_issuance_test;
mod balance_queries_test;
mod wallet_state_persistence_test;

/// Initialize logger for tests and load environment configuration
///
/// Sets up env_logger with INFO level to capture log output from
/// f1r3fly-rgb and f1r3fly-rgb-wallet crates during test execution.
///
/// Also loads environment variables from .env file, which is required for:
/// - FIREFLY_PRIVATE_KEY: Master key for F1r3node phlo payment (loaded into GlobalConfig)
/// - FIREFLY_HOST/PORTS: F1r3node connection details
///
/// Safe to call multiple times (subsequent calls are no-ops).
pub fn init_test_logger() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Info)
        .try_init();

    // Load environment variables from .env file in f1r3fly-rgb-wallet directory
    // This is required because GlobalConfig::default_regtest() reads FIREFLY_PRIVATE_KEY
    use std::path::PathBuf;
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(".env");
    dotenv::from_path(&path).ok();
}

/// Generate a unique starting derivation index for a test
///
/// Uses a hash of the test name to generate a deterministic but unique
/// derivation index. This prevents parallel tests from deploying contracts
/// to the same registry URI.
///
/// # Arguments
///
/// * `test_name` - Unique identifier for the test (e.g., function name)
///
/// # Returns
///
/// Derivation index in range [0, 16777216) (24-bit space for ~16M unique tests)
fn generate_test_derivation_index(test_name: &str) -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    test_name.hash(&mut hasher);
    let hash = hasher.finish();

    // Use 24 bits of hash (16M possible values) to avoid collision
    // while keeping index reasonable
    (hash % 16_777_216) as u32
}

/// Setup wallet with funded genesis UTXO for asset issuance
///
/// Creates a complete test environment:
/// 1. Creates wallet via WalletManager
/// 2. Funds wallet with Bitcoin from regtest
/// 3. Syncs wallet to detect funds
/// 4. Creates a UTXO via self-send for use as genesis seal
/// 5. Sets test-specific derivation index for contract isolation
/// 6. Returns manager and genesis UTXO string (format: "txid:vout")
///
/// # Arguments
///
/// * `env` - Test environment with regtest infrastructure
/// * `wallet_name` - Name for the test wallet
/// * `password` - Password to encrypt the wallet
///
/// # Returns
///
/// Tuple of (WalletManager, genesis_utxo_string)
///
/// # Panics
///
/// Panics if wallet creation, funding, or UTXO creation fails
///
/// # Example
///
/// ```ignore
/// let env = TestBitcoinEnv::new("test_issue_asset");
/// let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(
///     &env,
///     env.unique_wallet_name(),
///     "test_password",
/// ).unwrap();
/// ```
pub async fn setup_wallet_with_genesis_utxo(
    env: &TestBitcoinEnv,
    wallet_name: &str,
    password: &str,
) -> Result<(WalletManager, String), Box<dyn std::error::Error>> {
    // Initialize logger for test output
    init_test_logger();

    // Generate unique starting derivation index for this test
    // Uses wallet_name which contains the test name to ensure uniqueness
    let test_derivation_index = generate_test_derivation_index(wallet_name);
    
    // 1. Create wallet
    let mut manager = WalletManager::new(env.config().clone())?;
    manager.create_wallet(wallet_name, password)?;
    manager.load_wallet(wallet_name, password)?;

    // 1b. Set test-specific derivation index for contract isolation
    // This ensures parallel tests deploy to different registry URIs
    manager.set_f1r3fly_derivation_index(test_derivation_index)?;
    println!(
        "🔑 Test isolation: Using derivation index {} for test '{}'",
        test_derivation_index, wallet_name
    );

    // 2. Get first address for funding
    let addresses = manager.get_addresses(Some(1))?;
    let address = &addresses[0].address;

    // 3. Fund wallet from regtest
    println!("Funding address: {}", address);
    let address_str = address.to_string();
    let funding_txid = env.fund_address(&address_str, 1.0)?;
    println!("Funding txid: {}", funding_txid);

    // 4. Wait for confirmation
    env.wait_for_confirmation(&funding_txid, 1).await?;

    // 5. Sync wallet to detect funds
    manager.sync_wallet()?;

    // 6. Verify wallet has funds
    let balance = manager.get_balance()?;
    println!("Wallet balance after funding: {} sats", balance.confirmed);
    assert!(
        balance.confirmed > 0,
        "Wallet should have confirmed balance after funding"
    );

    // 7. Create UTXO for genesis seal via self-send
    // This creates a specific UTXO we can track
    // Mark as RGB-occupied immediately to reserve it and prevent it from being spent
    println!("Creating genesis UTXO via self-send...");
    let amount_sats = 1_000_000u64; // 0.01 BTC
    let fee_config = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
    let genesis_result = manager.create_utxo(amount_sats, &fee_config, true)?;
    let genesis_txid = genesis_result.txid.clone();
    println!("Genesis UTXO txid: {}", genesis_txid);

    // 8. Wait for genesis UTXO confirmation
    env.wait_for_confirmation(&genesis_txid, 1).await?;

    // 9. Sync wallet and wait for UTXO to be visible (with retries for Esplora indexing lag)
    for attempt in 1..=5 {
    manager.sync_wallet()?;
        
        // Check if the UTXO is now visible in the wallet
        let utxos: Vec<_> = manager.bitcoin_wallet().expect("Bitcoin wallet not loaded").inner().list_unspent().collect();
        let utxo_found = utxos.iter().any(|utxo| {
            utxo.outpoint.txid.to_string() == genesis_txid
        });
        
        if utxo_found {
            println!("✅ UTXO visible in wallet after {} sync attempt(s)", attempt);
            break;
        }
        
        if attempt < 5 {
            println!("⏳ UTXO not visible yet, waiting 1s before retry {} /5...", attempt + 1);
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        } else {
            println!("⚠️  Warning: UTXO still not visible after 5 sync attempts");
        }
    }

    // 10. Format genesis UTXO as "txid:vout" using the outpoint from the result
    // The create_utxo result contains the exact outpoint we need
    let genesis_utxo = format!(
        "{}:{}",
        genesis_result.outpoint.txid, genesis_result.outpoint.vout
    );
    println!(
        "Genesis UTXO: {} (amount: {} sats, vout: {})",
        genesis_utxo, genesis_result.amount, genesis_result.outpoint.vout
    );

    Ok((manager, genesis_utxo))
}

/// Check if F1r3node is available and reachable
///
/// Attempts to connect to F1r3node gRPC port to verify it's running.
/// Uses environment variables for host/port configuration, falling back to defaults.
///
/// # Returns
///
/// `true` if F1r3node is reachable, `false` otherwise
///
/// # Environment Variables
///
/// * `FIREFLY_HOST` or `FIREFLY_GRPC_HOST` - F1r3node gRPC host (default: "localhost")
/// * `FIREFLY_GRPC_PORT` - F1r3node gRPC port (default: "40401")
///
/// # Example
///
/// ```ignore
/// if !check_f1r3node_available() {
///     println!("Skipping test: F1r3node not available");
///     return;
/// }
/// ```
pub fn check_f1r3node_available() -> bool {
    // Initialize logger for tests that check F1r3node availability
    init_test_logger();
    
    let host = std::env::var("FIREFLY_HOST")
        .or_else(|_| std::env::var("FIREFLY_GRPC_HOST"))
        .unwrap_or_else(|_| "localhost".to_string());
    let port = std::env::var("FIREFLY_GRPC_PORT").unwrap_or_else(|_| "40401".to_string());
    let addr_str = format!("{}:{}", host, port);

    // Resolve hostname and try to connect with short timeout
    match addr_str.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(socket_addr) = addrs.next() {
                TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2)).is_ok()
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Assert F1r3node is available or skip test
///
/// Convenience macro for tests that require F1r3node.
/// Checks availability and returns early with a warning if not available.
///
/// # Example
///
/// ```ignore
/// #[test]
/// fn test_asset_issuance() {
///     require_f1r3node!();
///     // Test code...
/// }
/// ```
#[macro_export]
macro_rules! require_f1r3node {
    () => {
        if !$crate::f1r3fly::check_f1r3node_available() {
            println!("⚠️  Skipping test: F1r3node not available");
            println!("   Start F1r3node and set FIREFLY_* environment variables");
            return;
        }
    };
}
