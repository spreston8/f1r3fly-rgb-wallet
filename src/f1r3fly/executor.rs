//! F1r3fly Executor Manager
//!
//! Manages F1r3flyExecutor instances with wallet configuration and key management.
//! Creates executors with explicit connection config instead of environment variables.

use f1r3fly_rgb::F1r3flyExecutor;
use node_cli::connection_manager::{ConnectionConfig, F1r3flyConnectionManager};

use crate::config::GlobalConfig;
use crate::storage::models::WalletKeys;

/// Error type for F1r3fly executor operations
#[derive(Debug, thiserror::Error)]
pub enum F1r3flyExecutorError {
    /// Connection failed
    #[error("F1r3node connection failed: {0}")]
    ConnectionFailed(String),
}

/// Manages F1r3flyExecutor creation with wallet configuration
///
/// This struct provides a clean interface for creating F1r3flyExecutor instances
/// using the wallet's configuration and decrypted F1r3fly keys, avoiding the need
/// for environment variables.
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::f1r3fly::F1r3flyExecutorManager;
///
/// let manager = F1r3flyExecutorManager::new(&config, &wallet_keys)?;
/// let executor = manager.create_executor();
/// ```
pub struct F1r3flyExecutorManager {
    /// F1r3node connection manager
    connection: F1r3flyConnectionManager,
}

impl F1r3flyExecutorManager {
    /// Create a new executor manager
    ///
    /// Initializes the F1r3node connection using the wallet's global configuration
    /// and the decrypted F1r3fly private key.
    ///
    /// # Arguments
    ///
    /// * `config` - Global wallet configuration (contains F1r3node host/ports)
    /// * `wallet_keys` - Decrypted wallet keys (contains F1r3fly private key)
    ///
    /// # Returns
    ///
    /// New `F1r3flyExecutorManager` instance ready to create executors
    ///
    /// # Errors
    ///
    /// Returns error if F1r3node connection cannot be established
    pub fn new(
        config: &GlobalConfig,
        _wallet_keys: &WalletKeys,
    ) -> Result<Self, F1r3flyExecutorError> {
        // Use master key from configuration for phlo payment and gRPC signing
        // 
        // This key is used as:
        // - Master key: Signs gRPC deployments (pays phlo from its REV vault)
        // - Deployer identity: Public key appears in insertSigned as deployerPubKey
        // 
        // Child keys are derived from this master for unique contract URIs.
        // 
        // The wallet-derived F1r3fly key (from mnemonic) is reserved for future
        // wallet-specific features like authentication or namespacing.
        let f1r3fly_key_hex = config.f1r3node.master_key.clone();

        // Create connection configuration from wallet config
        let connection_config = ConnectionConfig::new(
            config.f1r3node.host.clone(),
            config.f1r3node.grpc_port,
            config.f1r3node.http_port,
            f1r3fly_key_hex,
        );

        // Create connection manager
        let connection = F1r3flyConnectionManager::new(connection_config);

        Ok(Self { connection })
    }

    /// Create a new F1r3flyExecutor instance
    ///
    /// Returns a new executor configured with this manager's connection.
    /// Each call creates a fresh executor with default settings (auto_derive = true).
    ///
    /// # Returns
    ///
    /// New `F1r3flyExecutor` instance ready for contract deployment and operations
    ///
    /// # Example
    ///
    /// ```ignore
    /// let executor = manager.create_executor();
    /// let contract_id = executor.deploy_contract(...).await?;
    /// ```
    pub fn create_executor(&self) -> F1r3flyExecutor {
        F1r3flyExecutor::with_connection(self.connection.clone())
    }

    /// Get reference to the underlying connection
    ///
    /// Useful for direct F1r3node queries or custom operations.
    pub fn connection(&self) -> &F1r3flyConnectionManager {
        &self.connection
    }
}

