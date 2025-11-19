//! F1r3fly-RGB Wallet Integration Test Helpers
//!
//! Shared test infrastructure for F1r3fly-RGB wallet layer integration tests.
//! These tests focus on wallet-specific wrappers and state persistence.

use f1r3fly_rgb_wallet::manager::WalletManager;
use f1r3fly_rgb_wallet::storage::file_system::wallet_exists;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

// Re-export common test utilities
pub use crate::common::TestBitcoinEnv;

// Test modules
mod asset_issuance_test;
mod balance_queries_test;
mod complete_transfer_test;
mod genesis_consignment_test;
mod invoice_operations_test;
mod multi_transfer_chain_test;
mod validation_security_test;
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

    // 1. Create or load wallet
    let mut manager = WalletManager::new(env.config().clone())?;
    let wallets_dir = env.config().wallets_dir.as_deref();

    if wallet_exists(wallet_name, wallets_dir) {
        println!("📂 Wallet '{}' already exists, loading it...", wallet_name);
        manager.load_wallet(wallet_name, password)?;
    } else {
        println!("🆕 Creating new wallet '{}'...", wallet_name);
        manager.create_wallet(wallet_name, password)?;
        manager.load_wallet(wallet_name, password)?;
    }

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
        let utxos: Vec<_> = manager
            .bitcoin_wallet()
            .expect("Bitcoin wallet not loaded")
            .inner()
            .list_unspent()
            .collect();
        let utxo_found = utxos
            .iter()
            .any(|utxo| utxo.outpoint.txid.to_string() == genesis_txid);

        if utxo_found {
            println!(
                "✅ UTXO visible in wallet after {} sync attempt(s)",
                attempt
            );
            break;
        }

        if attempt < 5 {
            println!(
                "⏳ UTXO not visible yet, waiting 1s before retry {} /5...",
                attempt + 1
            );
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

/// Setup recipient wallet (funded, no assets issued)
///
/// Creates a wallet with Bitcoin funds but no RGB assets.
/// Used for creating transfer recipients in tests.
///
/// # Arguments
///
/// * `env` - Test environment with regtest infrastructure
/// * `wallet_name` - Name for the recipient wallet
/// * `password` - Password to encrypt the wallet
///
/// # Returns
///
/// `WalletManager` ready to receive RGB transfers
///
/// # Example
///
/// ```ignore
/// let env = TestBitcoinEnv::new("transfer_test");
/// let mut bob = setup_recipient_wallet(&env, "bob", "password").await?;
/// ```
pub async fn setup_recipient_wallet(
    env: &TestBitcoinEnv,
    wallet_name: &str,
    password: &str,
) -> Result<WalletManager, Box<dyn std::error::Error>> {
    init_test_logger();

    log::debug!("Setting up recipient wallet: {}", wallet_name);

    // Generate unique derivation index for test isolation
    let test_derivation_index = generate_test_derivation_index(wallet_name);

    // Create and load wallet
    let mut manager = WalletManager::new(env.config().clone())?;
    manager.create_wallet(wallet_name, password)?;
    manager.load_wallet(wallet_name, password)?;

    // Set derivation index for contract isolation
    manager.set_f1r3fly_derivation_index(test_derivation_index)?;

    // Fund wallet
    let addresses = manager.get_addresses(Some(1))?;
    let address = &addresses[0].address.to_string();

    log::debug!("Funding recipient address: {}", address);
    let funding_txid = env.fund_address(address, 1.0)?;
    env.wait_for_confirmation(&funding_txid, 1).await?;

    // Sync to detect funds
    manager.sync_wallet()?;

    let balance = manager.get_balance()?;
    log::debug!(
        "Recipient wallet {} funded with {} sats",
        wallet_name,
        balance.confirmed
    );

    Ok(manager)
}

/// Issue test asset with common parameters
///
/// Wrapper around wallet setup and asset issuance for cleaner test code.
/// Creates wallet, funds it, issues asset, and returns all necessary info.
///
/// # Arguments
///
/// * `env` - Test environment
/// * `wallet_name` - Name for the issuer wallet
/// * `ticker` - Asset ticker
/// * `supply` - Total supply
///
/// # Returns
///
/// Tuple of (WalletManager, AssetInfo, genesis_utxo)
///
/// # Example
///
/// ```ignore
/// let (mut alice, asset_info, genesis_utxo) =
///     issue_test_asset(&env, "alice", "TEST", 10_000).await?;
/// ```
pub async fn issue_test_asset(
    env: &TestBitcoinEnv,
    wallet_name: &str,
    ticker: &str,
    supply: u64,
) -> Result<
    (
        WalletManager,
        f1r3fly_rgb_wallet::f1r3fly::AssetInfo,
        String,
    ),
    Box<dyn std::error::Error>,
> {
    log::debug!(
        "Issuing test asset: {} ({} units) for wallet {}",
        ticker,
        supply,
        wallet_name
    );

    // Setup wallet with genesis UTXO
    let (mut manager, genesis_utxo) =
        setup_wallet_with_genesis_utxo(env, wallet_name, "test_password").await?;

    // Issue asset with standard test parameters
    let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: ticker.to_string(),
        name: format!("{} Token", ticker),
        supply,
        precision: 8,
        genesis_utxo: genesis_utxo.clone(),
    };

    let asset_info = manager.issue_asset(request).await?;

    log::debug!(
        "Asset issued: {} (contract_id: {})",
        ticker,
        asset_info.contract_id
    );

    Ok((manager, asset_info, genesis_utxo))
}

/// Verify consignment file exists and has valid structure
///
/// Checks that consignment file:
/// - Exists on disk
/// - Is valid JSON
/// - Has expected size range (5-50 KB)
/// - Contains required fields
///
/// # Arguments
///
/// * `path` - Path to consignment file
///
/// # Returns
///
/// Ok if valid, Err with description otherwise
pub fn verify_consignment_file(path: &std::path::Path) -> Result<(), String> {
    // Check file exists
    if !path.exists() {
        return Err(format!("Consignment file not found: {}", path.display()));
    }

    // Read and parse as JSON
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Verify size (5 KB - 50 KB is reasonable for consignments)
    let size = bytes.len();
    if size < 1024 {
        return Err(format!("Consignment too small: {} bytes", size));
    }
    if size > 52_428_800 {
        // 50 MB
        return Err(format!("Consignment too large: {} bytes", size));
    }

    log::debug!("Consignment file size: {} bytes", size);

    // Verify it's valid JSON
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid JSON: {}", e))?;

    // Check for required top-level fields
    let required_fields = [
        "version",
        "contract_id",
        "f1r3fly_proof",
        "bitcoin_anchor",
        "seals",
    ];
    for field in &required_fields {
        if json.get(field).is_none() {
            return Err(format!("Missing required field: {}", field));
        }
    }

    log::debug!("Consignment file valid: {}", path.display());
    Ok(())
}

/// Verify balance with retry logic for F1r3fly state propagation
///
/// F1r3fly state updates may take time to propagate.
/// This helper retries balance queries with delays to handle timing issues.
///
/// # Arguments
///
/// * `manager` - Wallet manager to query
/// * `contract_id` - Contract ID to check balance for
/// * `expected` - Expected balance amount
/// * `max_attempts` - Maximum retry attempts (default: 10)
///
/// # Returns
///
/// Ok if balance matches within retry limit, Err otherwise
///
/// # Example
///
/// ```ignore
/// verify_balance_with_retry(&mut alice, &contract_id, 7500, 5).await?;
/// ```
pub async fn verify_balance_with_retry(
    manager: &mut WalletManager,
    contract_id: &str,
    expected: u64,
    max_attempts: u32,
) -> Result<(), String> {
    use tokio::time::{sleep, Duration};

    log::debug!(
        "Verifying balance for contract {} (expected: {}, max_attempts: {})",
        contract_id,
        expected,
        max_attempts
    );

    for attempt in 1..=max_attempts {
        match manager.get_asset_balance(contract_id).await {
            Ok(balance) => {
                if balance.total == expected {
                    log::debug!(
                        "✓ Balance verified: {} (attempt {}/{})",
                        expected,
                        attempt,
                        max_attempts
                    );
                    return Ok(());
                } else {
                    log::debug!(
                        "Balance mismatch: got {}, expected {} (attempt {}/{})",
                        balance.total,
                        expected,
                        attempt,
                        max_attempts
                    );
                }
            }
            Err(e) => {
                log::debug!(
                    "Balance query failed: {} (attempt {}/{})",
                    e,
                    attempt,
                    max_attempts
                );
            }
        }

        if attempt < max_attempts {
            sleep(Duration::from_secs(1)).await;
        }
    }

    Err(format!(
        "Balance verification failed after {} attempts for contract {}",
        max_attempts, contract_id
    ))
}

/// Multiple test wallets for multi-party scenarios
///
/// Pre-configured wallets for common test scenarios involving
/// multiple parties (Alice, Bob, Carol).
pub struct TestWallets {
    /// Alice's wallet (typically the issuer)
    pub alice: WalletManager,

    /// Bob's wallet (typically first recipient)
    pub bob: WalletManager,

    /// Carol's wallet (typically second recipient)
    pub carol: WalletManager,
}

/// Setup three funded wallets for multi-party tests
///
/// Creates Alice, Bob, and Carol wallets, all funded with Bitcoin.
/// Useful for testing complex transfer chains and multi-party scenarios.
///
/// # Arguments
///
/// * `env` - Test environment
///
/// # Returns
///
/// `TestWallets` with three ready-to-use wallets
///
/// # Example
///
/// ```ignore
/// let env = TestBitcoinEnv::new("multi_party_test");
/// let wallets = setup_test_wallets(&env).await?;
///
/// // Use wallets
/// let asset_info = wallets.alice.issue_asset(...).await?;
/// ```
pub async fn setup_test_wallets(
    env: &TestBitcoinEnv,
) -> Result<TestWallets, Box<dyn std::error::Error>> {
    log::debug!("Setting up test wallets: Alice, Bob, Carol");

    // Create unique names to avoid conflicts
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let alice_name = format!("alice_{}", timestamp);
    let bob_name = format!("bob_{}", timestamp);
    let carol_name = format!("carol_{}", timestamp);

    // Setup all three wallets in parallel for speed
    let (alice_result, bob_result, carol_result) = tokio::join!(
        setup_recipient_wallet(env, &alice_name, "password"),
        setup_recipient_wallet(env, &bob_name, "password"),
        setup_recipient_wallet(env, &carol_name, "password"),
    );

    log::debug!("All test wallets created");

    Ok(TestWallets {
        alice: alice_result?,
        bob: bob_result?,
        carol: carol_result?,
    })
}
