//! Bitcoin network layer using Esplora client

use crate::config::NetworkType;
use bdk_esplora::esplora_client::{self, BlockingClient};
use std::time::Duration;

/// Errors that can occur during network operations
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Esplora client error: {0}")]
    Esplora(#[from] esplora_client::Error),

    #[error("Network request failed: {0}")]
    Request(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Connection timeout")]
    Timeout,

    #[error("Network unavailable")]
    Unavailable,
}

/// Esplora client wrapper for blockchain queries
///
/// Provides a blocking interface to query blockchain data from an Esplora server.
/// Configured with network-specific endpoints (local regtest, public signet/testnet/mainnet).
pub struct EsploraClient {
    /// Underlying Esplora blocking client
    client: BlockingClient,

    /// Network type
    network: NetworkType,

    /// Esplora server URL
    url: String,
}

impl EsploraClient {
    /// Create a new Esplora client with custom URL
    ///
    /// # Arguments
    ///
    /// * `url` - Esplora server URL (e.g., "http://localhost:3002")
    /// * `network` - Network type
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::bitcoin::EsploraClient;
    /// use f1r3fly_rgb_wallet::config::NetworkType;
    ///
    /// let client = EsploraClient::new(
    ///     "http://localhost:3002",
    ///     NetworkType::Regtest,
    /// )?;
    /// ```
    pub fn new(url: &str, network: NetworkType) -> Result<Self, NetworkError> {
        let builder = esplora_client::Builder::new(url);
        let client = BlockingClient::from_builder(builder);

        Ok(Self {
            client,
            network,
            url: url.to_string(),
        })
    }

    /// Create a new Esplora client with custom timeout
    ///
    /// # Arguments
    ///
    /// * `url` - Esplora server URL
    /// * `network` - Network type
    /// * `timeout` - Request timeout duration
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::time::Duration;
    ///
    /// let client = EsploraClient::with_timeout(
    ///     "http://localhost:3002",
    ///     NetworkType::Regtest,
    ///     Duration::from_secs(60),
    /// )?;
    /// ```
    pub fn with_timeout(
        url: &str,
        network: NetworkType,
        timeout: Duration,
    ) -> Result<Self, NetworkError> {
        let builder = esplora_client::Builder::new(url)
            .timeout(timeout.as_secs());
        let client = BlockingClient::from_builder(builder);

        Ok(Self {
            client,
            network,
            url: url.to_string(),
        })
    }

    /// Create a new Esplora client with default network-specific URL
    ///
    /// Uses predefined URLs for each network:
    /// - Regtest: http://localhost:3002
    /// - Signet: https://mempool.space/signet/api
    /// - Testnet: https://mempool.space/testnet/api
    /// - Mainnet: https://mempool.space/api
    ///
    /// # Arguments
    ///
    /// * `network` - Network type
    ///
    /// # Example
    ///
    /// ```ignore
    /// let client = EsploraClient::new_with_default_url(NetworkType::Regtest)?;
    /// ```
    pub fn new_with_default_url(network: NetworkType) -> Result<Self, NetworkError> {
        let url = default_esplora_url(network);
        Self::new(&url, network)
    }

    /// Get the underlying Esplora client reference
    pub fn inner(&self) -> &BlockingClient {
        &self.client
    }

    /// Get mutable reference to the underlying Esplora client
    pub fn inner_mut(&mut self) -> &mut BlockingClient {
        &mut self.client
    }

    /// Get the network type
    pub fn network(&self) -> NetworkType {
        self.network
    }

    /// Get the Esplora server URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Get the current blockchain tip height
    ///
    /// # Example
    ///
    /// ```ignore
    /// let height = client.get_height()?;
    /// println!("Current block height: {}", height);
    /// ```
    pub fn get_height(&self) -> Result<u32, NetworkError> {
        self.client
            .get_height()
            .map_err(|e| NetworkError::Request(format!("Failed to get height: {}", e)))
    }

    /// Get the current blockchain tip block hash
    ///
    /// # Example
    ///
    /// ```ignore
    /// let tip_hash = client.get_tip_hash()?;
    /// println!("Current tip hash: {}", tip_hash);
    /// ```
    pub fn get_tip_hash(&self) -> Result<bdk_wallet::bitcoin::BlockHash, NetworkError> {
        self.client
            .get_tip_hash()
            .map_err(|e| NetworkError::Request(format!("Failed to get tip hash: {}", e)))
    }

    /// Check if the Esplora server is reachable
    ///
    /// Attempts to query the current height to verify connectivity.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if client.is_available()? {
    ///     println!("Esplora server is available");
    /// }
    /// ```
    pub fn is_available(&self) -> Result<bool, NetworkError> {
        match self.get_height() {
            Ok(_) => Ok(true),
            Err(NetworkError::Request(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Get default Esplora URL for a given network
///
/// # Returns
///
/// - Regtest: `http://localhost:3002`
/// - Signet: `https://mempool.space/signet/api`
/// - Testnet: `https://mempool.space/testnet/api`
/// - Mainnet: `https://mempool.space/api`
pub fn default_esplora_url(network: NetworkType) -> String {
    match network {
        NetworkType::Regtest => "http://localhost:3002".to_string(),
        NetworkType::Signet => "https://mempool.space/signet/api".to_string(),
        NetworkType::Testnet => "https://mempool.space/testnet/api".to_string(),
        NetworkType::Mainnet => "https://mempool.space/api".to_string(),
    }
}

impl std::fmt::Debug for EsploraClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EsploraClient")
            .field("network", &self.network)
            .field("url", &self.url)
            .finish()
    }
}

