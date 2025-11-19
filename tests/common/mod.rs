//! Common test utilities for f1r3fly-rgb-wallet Bitcoin layer integration tests
//!
//! This module provides shared test infrastructure including:
//! - Test environment setup with automatic cleanup
//! - Bitcoin RPC client for mining and funding
//! - Regtest connectivity checks
//! - Helper functions for confirmations and sync

use f1r3fly_rgb_wallet::bitcoin::network::EsploraClient;
use f1r3fly_rgb_wallet::config::GlobalConfig;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// Bitcoin RPC client for test operations
///
/// Provides methods to interact with Bitcoin Core regtest via bitcoin-cli
pub struct BitcoinRpcClient {
    pub datadir: String,
}

impl BitcoinRpcClient {
    /// Create new Bitcoin RPC client
    ///
    /// Uses BITCOIN_DATADIR environment variable or defaults to project root .bitcoin
    pub fn new() -> Self {
        let datadir_path = std::env::var("BITCOIN_DATADIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                // Default to project root .bitcoin directory
                let current_dir = std::env::current_dir().expect("Failed to get current directory");
                current_dir
                    .parent()
                    .expect("Failed to get parent directory")
                    .join(".bitcoin")
            });

        let datadir = datadir_path.to_string_lossy().to_string();

        Self { datadir }
    }

    /// Execute bitcoin-cli command
    ///
    /// Supports both local and Docker environments:
    /// - If Bitcoin is in Docker (CI or docker-compose), use docker exec
    /// - Otherwise, use local bitcoin-cli with datadir
    fn execute_cli(&self, args: &[&str]) -> Result<String, String> {
        // Check if we should use Docker (bitcoin-test container exists)
        let use_docker = Command::new("docker")
            .args([
                "ps",
                "--filter",
                "name=bitcoind-test",
                "--format",
                "{{.Names}}",
            ])
            .output()
            .ok()
            .and_then(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Some(stdout.contains("bitcoind-test"))
            })
            .unwrap_or(false);

        let output = if use_docker {
            // Use docker exec to run bitcoin-cli inside the container
            let mut cmd = Command::new("docker");
            cmd.args([
                "exec",
                "bitcoind-test",
                "bitcoin-cli",
                "-regtest",
                "-rpcuser=user",
                "-rpcpassword=password",
            ]);
            for arg in args {
                cmd.arg(arg);
            }
            cmd.output()
                .map_err(|e| format!("Failed to execute docker exec bitcoin-cli: {}", e))?
        } else {
            // Use local bitcoin-cli with datadir
            let mut cmd = Command::new("bitcoin-cli");
            cmd.arg("-regtest")
                .arg(format!("-datadir={}", self.datadir));
            for arg in args {
                cmd.arg(arg);
            }
            cmd.output()
                .map_err(|e| format!("Failed to execute bitcoin-cli: {}", e))?
        };

        if !output.status.success() {
            return Err(format!(
                "bitcoin-cli failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Mine blocks to specified address
    ///
    /// # Arguments
    ///
    /// * `count` - Number of blocks to mine
    /// * `address` - Address to receive mining rewards
    ///
    /// # Returns
    ///
    /// Vector of block hashes
    pub fn mine_to_address(&self, count: u32, address: &str) -> Result<Vec<String>, String> {
        let result = self.execute_cli(&["generatetoaddress", &count.to_string(), address])?;

        // Parse JSON array of block hashes
        let hashes: Vec<String> = serde_json::from_str(&result)
            .map_err(|e| format!("Failed to parse block hashes: {}", e))?;

        Ok(hashes)
    }

    /// Send Bitcoin from mining wallet to address
    ///
    /// # Arguments
    ///
    /// * `address` - Destination address
    /// * `amount_btc` - Amount in BTC
    ///
    /// # Returns
    ///
    /// Transaction ID
    pub fn send_to_address(&self, address: &str, amount_btc: f64) -> Result<String, String> {
        let txid = self.execute_cli(&[
            "-rpcwallet=mining_wallet",
            "sendtoaddress",
            address,
            &amount_btc.to_string(),
        ])?;

        Ok(txid)
    }

    /// Get transaction details
    /// Get current blockchain height
    ///
    /// # Returns
    ///
    /// Current block count
    pub fn get_block_count(&self) -> Result<u32, String> {
        let count = self.execute_cli(&["getblockcount"])?;

        count
            .parse::<u32>()
            .map_err(|e| format!("Failed to parse block count: {}", e))
    }

    /// Check if bitcoin-cli is available and responsive
    pub fn is_available(&self) -> bool {
        self.get_block_count().is_ok()
    }
}

/// Test Bitcoin environment with automatic cleanup
///
/// Provides isolated test environment with unique wallet names,
/// temporary directories, and helper methods for Bitcoin operations.
pub struct TestBitcoinEnv {
    /// Temporary directory (auto-cleanup on drop)
    _temp_dir: TempDir,

    /// Wallets directory path
    wallets_dir: PathBuf,

    /// Global config (regtest)
    config: GlobalConfig,

    /// Esplora client for regtest  
    pub esplora_client: EsploraClient,

    /// Unique test wallet name
    test_wallet_name: String,

    /// Bitcoin RPC client
    bitcoin_rpc: BitcoinRpcClient,
}

impl TestBitcoinEnv {
    /// Create new test environment
    ///
    /// # Arguments
    ///
    /// * `test_name` - Name of the test (used to generate unique wallet name)
    ///
    /// # Panics
    ///
    /// Panics if regtest environment is not running
    pub fn new(test_name: &str) -> Self {
        // Load .env file for environment variables (like FIREFLY_PRIVATE_KEY)
        use std::path::PathBuf;
        let mut env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        env_path.push(".env");
        dotenv::from_path(&env_path).ok();

        // Check regtest is running
        Self::check_regtest_running()
            .expect("Regtest environment is not running. Run ./scripts/start-regtest.sh");

        // Create temp directory
        let temp_dir = TempDir::new().expect("Failed to create temporary directory for test");
        let wallets_dir = temp_dir.path().join("wallets");
        std::fs::create_dir_all(&wallets_dir).expect("Failed to create wallets directory");

        // Create regtest config with custom wallets directory
        let mut config = Self::regtest_config();
        config.wallets_dir = Some(wallets_dir.to_string_lossy().to_string());

        // Generate unique wallet name with timestamp and UUID
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Failed to get system time")
            .as_secs();
        let uuid = uuid::Uuid::new_v4();
        let test_wallet_name = format!("test-{}-{}-{}", test_name, timestamp, uuid);

        // Create Bitcoin RPC client
        let bitcoin_rpc = BitcoinRpcClient::new();

        // Create Esplora client
        let esplora_client =
            EsploraClient::new(&config.bitcoin.esplora_url, config.bitcoin.network)
                .expect("Failed to create Esplora client");

        Self {
            _temp_dir: temp_dir,
            wallets_dir,
            config,
            esplora_client,
            test_wallet_name,
            bitcoin_rpc,
        }
    }

    /// Check if regtest environment is running
    ///
    /// Verifies that Bitcoin Core is reachable via bitcoin-cli.
    ///
    /// Note: Esplora connectivity is verified dynamically by the actual
    /// API calls (e.g., wait_for_confirmation) which will fail with clear
    /// errors if Esplora is unavailable.
    ///
    /// # Returns
    ///
    /// Ok if Bitcoin Core is running, Err with message otherwise
    fn check_regtest_running() -> Result<(), String> {
        // Check Bitcoin Core
        let bitcoin_rpc = BitcoinRpcClient::new();
        if !bitcoin_rpc.is_available() {
            return Err(
                "Bitcoin Core not responding. Please run: ./scripts/start-regtest.sh".to_string(),
            );
        }

        Ok(())
    }

    /// Get regtest configuration
    ///
    /// # Returns
    ///
    /// GlobalConfig configured for regtest with localhost:3002 Esplora
    fn regtest_config() -> GlobalConfig {
        GlobalConfig::default_regtest()
    }

    /// Get unique wallet name for this test
    pub fn unique_wallet_name(&self) -> &str {
        &self.test_wallet_name
    }

    /// Get wallet directory path
    pub fn wallet_dir(&self, name: &str) -> PathBuf {
        self.wallets_dir.join(name)
    }

    /// Get global config
    pub fn config(&self) -> &GlobalConfig {
        &self.config
    }

    /// Mine N blocks to default mining address
    ///
    /// # Arguments
    ///
    /// * `count` - Number of blocks to mine
    ///
    /// # Returns
    ///
    /// Vector of block hashes
    pub fn mine_blocks(&self, count: u32) -> Result<Vec<String>, String> {
        // Get mining address from environment or use a default
        let mining_address = std::env::var("MINING_ADDRESS")
            .unwrap_or_else(|_| "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080".to_string());

        let hashes = self.bitcoin_rpc.mine_to_address(count, &mining_address)?;

        // Wait for Electrs to index
        if count > 0 {
            thread::sleep(Duration::from_millis(1000));
        }

        Ok(hashes)
    }

    /// Send BTC from regtest mining wallet to address
    ///
    /// # Arguments
    ///
    /// * `address` - Destination address
    /// * `btc_amount` - Amount in BTC
    ///
    /// # Returns
    ///
    /// Transaction ID
    pub fn fund_address(&self, address: &str, btc_amount: f64) -> Result<String, String> {
        self.bitcoin_rpc.send_to_address(address, btc_amount)
    }

    /// Wait for transaction to be confirmed
    ///
    /// Mines blocks and polls Esplora (via BDK) to verify the transaction
    /// is confirmed. This is production-ready with proper retry logic.
    ///
    /// # Arguments
    ///
    /// * `txid` - Transaction ID
    /// * `confirmations` - Number of confirmations to wait for
    ///
    /// # Returns
    ///
    /// Block height where transaction was confirmed
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Mining fails
    /// - Transaction not found after retries
    /// - Esplora sync fails
    pub async fn wait_for_confirmation(
        &self,
        txid: &str,
        confirmations: u32,
    ) -> Result<u32, String> {
        use f1r3fly_rgb_wallet::bitcoin::network::EsploraClient;

        if confirmations == 0 {
            return Err("Confirmations must be at least 1".to_string());
        }

        // Mine the required number of blocks
        self.mine_blocks(confirmations)?;
        let height = self.bitcoin_rpc.get_block_count()?;

        // Poll Esplora API to verify transaction is indexed
        // Use the EsploraClient which is designed for this
        let esplora = EsploraClient::new(
            "http://localhost:3002",
            f1r3fly_rgb_wallet::config::NetworkType::Regtest,
        )
        .map_err(|e| format!("Failed to create Esplora client: {}", e))?;

        // Parse txid
        let txid_parsed: bdk_wallet::bitcoin::Txid =
            txid.parse().map_err(|e| format!("Invalid txid: {}", e))?;

        // Poll with retries (generous timeout for CI environments)
        // Esplora indexing can be slow under load, especially in CI
        let max_retries = 60; // Increased from 30 for flaky CI environments
        let retry_delay = Duration::from_millis(500); // Increased from 300ms

        for attempt in 1..=max_retries {
            tokio::time::sleep(retry_delay).await;

            // Query transaction via Esplora blocking API (run in spawn_blocking to avoid runtime issues)
            let txid_for_thread = txid_parsed;
            let esplora_inner = esplora.inner().clone();

            let tx_status =
                tokio::task::spawn_blocking(move || esplora_inner.get_tx_status(&txid_for_thread))
                    .await
                    .map_err(|e| format!("Task join error: {}", e))?
                    .map_err(|e| format!("Esplora query error: {}", e))?;

            if tx_status.confirmed {
                if let Some(block_height) = tx_status.block_height {
                    return Ok(block_height);
                }
            }

            if attempt == max_retries {
                return Err(format!(
                    "Transaction {} not confirmed after {} attempts",
                    txid, max_retries
                ));
            }
        }

        Ok(height)
    }

    /// Get current blockchain height
    ///
    /// # Returns
    ///
    /// Current block height
    pub fn get_blockchain_height(&self) -> Result<u32, String> {
        self.bitcoin_rpc.get_block_count()
    }

    /// Generate new regtest address for testing
    ///
    /// # Returns
    ///
    /// New regtest address from mining wallet
    pub fn get_new_test_address(&self) -> Result<String, String> {
        self.bitcoin_rpc
            .execute_cli(&["-rpcwallet=mining_wallet", "getnewaddress", "", "bech32"])
    }

    /// Get Bitcoin RPC client
    pub fn bitcoin_rpc(&self) -> &BitcoinRpcClient {
        &self.bitcoin_rpc
    }
}

impl Drop for TestBitcoinEnv {
    /// Cleanup temporary directories
    ///
    /// Note: Bitcoin regtest state persists (by design, it's a shared resource)
    fn drop(&mut self) {
        // TempDir auto-cleanup handles wallet directory removal

        // Suppress "never read/used" warnings - these ARE used in test files
        // but Rust's linter checks modules in isolation
        let _ = &self.esplora_client;
        let _ = self.get_blockchain_height();
        let _ = self.get_new_test_address();
        let _ = self.bitcoin_rpc();
    }
}
