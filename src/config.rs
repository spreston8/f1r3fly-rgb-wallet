//! Configuration types for F1r3fly-RGB wallet
//!
//! Manages global configuration including network settings, F1r3node connection,
//! and Bitcoin/Esplora endpoints.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Global wallet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub f1r3node: F1r3nodeConfig,
    pub bitcoin: BitcoinConfig,
    /// Optional custom wallets directory
    pub wallets_dir: Option<String>,
}

/// F1r3node connection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct F1r3nodeConfig {
    pub host: String,
    pub grpc_port: u16,
    pub http_port: u16,
    /// Master private key for F1r3node operations (hex-encoded)
    ///
    /// This key is used for:
    /// - Signing gRPC deployments (phlo payment)
    /// - Serving as the deployer identity in insertSigned
    ///
    /// Must be a funded key with sufficient REV balance.
    /// Typically loaded from FIREFLY_PRIVATE_KEY environment variable.
    pub master_key: String,
}

/// Bitcoin network and blockchain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoinConfig {
    pub network: NetworkType,
    pub esplora_url: String,
}

/// Bitcoin network type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkType {
    Regtest,
    Signet,
    Testnet,
    Mainnet,
}

impl GlobalConfig {
    /// Create default configuration for regtest
    pub fn default_regtest() -> Self {
        Self {
            f1r3node: F1r3nodeConfig {
                host: "localhost".to_string(),
                grpc_port: 40401,
                http_port: 40403,
                master_key: std::env::var("FIREFLY_PRIVATE_KEY")
                    .expect("FIREFLY_PRIVATE_KEY environment variable must be set"),
            },
            bitcoin: BitcoinConfig {
                network: NetworkType::Regtest,
                esplora_url: "http://localhost:3002".to_string(),
            },
            wallets_dir: None,
        }
    }

    /// Create default configuration for signet
    pub fn default_signet() -> Self {
        Self {
            f1r3node: F1r3nodeConfig {
                host: "localhost".to_string(),
                grpc_port: 40401,
                http_port: 40403,
                master_key: std::env::var("FIREFLY_PRIVATE_KEY")
                    .expect("FIREFLY_PRIVATE_KEY environment variable must be set"),
            },
            bitcoin: BitcoinConfig {
                network: NetworkType::Signet,
                esplora_url: "https://mempool.space/signet/api".to_string(),
            },
            wallets_dir: None,
        }
    }

    /// Create default configuration for testnet
    pub fn default_testnet() -> Self {
        Self {
            f1r3node: F1r3nodeConfig {
                host: "localhost".to_string(),
                grpc_port: 40401,
                http_port: 40403,
                master_key: std::env::var("FIREFLY_PRIVATE_KEY")
                    .expect("FIREFLY_PRIVATE_KEY environment variable must be set"),
            },
            bitcoin: BitcoinConfig {
                network: NetworkType::Testnet,
                esplora_url: "https://mempool.space/testnet/api".to_string(),
            },
            wallets_dir: None,
        }
    }

    /// Create default configuration for mainnet
    pub fn default_mainnet() -> Self {
        Self {
            f1r3node: F1r3nodeConfig {
                host: "localhost".to_string(),
                grpc_port: 40401,
                http_port: 40403,
                master_key: std::env::var("FIREFLY_PRIVATE_KEY")
                    .expect("FIREFLY_PRIVATE_KEY environment variable must be set"),
            },
            bitcoin: BitcoinConfig {
                network: NetworkType::Mainnet,
                esplora_url: "https://mempool.space/api".to_string(),
            },
            wallets_dir: None,
        }
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self::default_regtest()
    }
}

/// Configuration error types
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Invalid network: {0}")]
    InvalidNetwork(String),

    #[error("Config directory not found")]
    DirectoryNotFound,
}

/// Configuration overrides from CLI arguments or environment variables
#[derive(Debug, Default, Clone)]
pub struct ConfigOverrides {
    pub network: Option<NetworkType>,
    pub f1r3node_host: Option<String>,
    pub f1r3node_grpc_port: Option<u16>,
    pub f1r3node_http_port: Option<u16>,
    pub esplora_url: Option<String>,
    pub wallets_dir: Option<String>,
}

impl ConfigOverrides {
    /// Create empty overrides
    pub fn new() -> Self {
        Self::default()
    }

    /// Create overrides from environment variables
    ///
    /// Supports both FIREFLY_* and F1R3NODE_* prefixes for backwards compatibility.
    /// FIREFLY_* takes precedence if both are set.
    pub fn from_env() -> Self {
        Self {
            network: std::env::var("BITCOIN_NETWORK").ok().and_then(|s| {
                match s.to_lowercase().as_str() {
                    "regtest" => Some(NetworkType::Regtest),
                    "signet" => Some(NetworkType::Signet),
                    "testnet" => Some(NetworkType::Testnet),
                    "mainnet" => Some(NetworkType::Mainnet),
                    _ => None,
                }
            }),
            f1r3node_host: std::env::var("FIREFLY_HOST")
                .or_else(|_| std::env::var("FIREFLY_GRPC_HOST"))
                .or_else(|_| std::env::var("F1R3NODE_HOST"))
                .ok(),
            f1r3node_grpc_port: std::env::var("FIREFLY_GRPC_PORT")
                .or_else(|_| std::env::var("F1R3NODE_GRPC_PORT"))
                .ok()
                .and_then(|s| s.parse().ok()),
            f1r3node_http_port: std::env::var("FIREFLY_HTTP_PORT")
                .or_else(|_| std::env::var("F1R3NODE_HTTP_PORT"))
                .ok()
                .and_then(|s| s.parse().ok()),
            esplora_url: std::env::var("ESPLORA_URL").ok(),
            wallets_dir: std::env::var("WALLETS_DIR").ok(),
        }
    }

    /// Merge with another set of overrides (other takes precedence)
    pub fn merge(mut self, other: Self) -> Self {
        if other.network.is_some() {
            self.network = other.network;
        }
        if other.f1r3node_host.is_some() {
            self.f1r3node_host = other.f1r3node_host;
        }
        if other.f1r3node_grpc_port.is_some() {
            self.f1r3node_grpc_port = other.f1r3node_grpc_port;
        }
        if other.f1r3node_http_port.is_some() {
            self.f1r3node_http_port = other.f1r3node_http_port;
        }
        if other.esplora_url.is_some() {
            self.esplora_url = other.esplora_url;
        }
        self
    }
}

/// Get the default configuration directory path
///
/// Returns: `~/.f1r3fly-rgb-wallet/`
pub fn default_config_dir() -> Result<PathBuf, ConfigError> {
    dirs::home_dir()
        .map(|home| home.join(".f1r3fly-rgb-wallet"))
        .ok_or(ConfigError::DirectoryNotFound)
}

/// Get the default configuration file path
///
/// Returns: `~/.f1r3fly-rgb-wallet/config.json`
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    Ok(default_config_dir()?.join("config.json"))
}

/// Load configuration from file with overrides
///
/// # Priority (highest to lowest):
/// 1. CLI overrides (passed as argument)
/// 2. Environment variables
/// 3. Config file
/// 4. Network defaults
///
/// # Arguments
///
/// * `config_path` - Path to config file (optional, uses default if None)
/// * `cli_overrides` - Overrides from CLI arguments
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::config::{load_config, ConfigOverrides};
///
/// let mut cli_overrides = ConfigOverrides::new();
/// cli_overrides.network = Some(NetworkType::Regtest);
///
/// let config = load_config(None, cli_overrides)?;
/// ```
pub fn load_config(
    config_path: Option<&Path>,
    cli_overrides: ConfigOverrides,
) -> Result<GlobalConfig, ConfigError> {
    // Determine config path
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => default_config_path()?,
    };

    // Start with network defaults
    let mut config = if path.exists() {
        // Load from file if it exists
        let contents = std::fs::read_to_string(&path)?;
        serde_json::from_str(&contents)?
    } else {
        // Use network default (regtest by default, or from overrides)
        match cli_overrides.network {
            Some(NetworkType::Mainnet) => GlobalConfig::default_mainnet(),
            Some(NetworkType::Testnet) => GlobalConfig::default_testnet(),
            Some(NetworkType::Signet) => GlobalConfig::default_signet(),
            _ => GlobalConfig::default_regtest(),
        }
    };

    // Apply environment variable overrides
    let env_overrides = ConfigOverrides::from_env();
    apply_overrides(&mut config, env_overrides);

    // Apply CLI overrides (highest priority)
    apply_overrides(&mut config, cli_overrides);

    Ok(config)
}

/// Save configuration to file
///
/// Creates parent directories if they don't exist.
///
/// # Arguments
///
/// * `config` - Configuration to save
/// * `config_path` - Path to save config (optional, uses default if None)
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::config::{save_config, GlobalConfig};
///
/// let config = GlobalConfig::default_regtest();
/// save_config(&config, None)?;
/// ```
pub fn save_config(config: &GlobalConfig, config_path: Option<&Path>) -> Result<(), ConfigError> {
    // Determine config path
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => default_config_path()?,
    };

    // Create parent directory if it doesn't exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Serialize to pretty JSON
    let json = serde_json::to_string_pretty(config)?;

    // Write to file
    std::fs::write(&path, json)?;

    Ok(())
}

/// Apply configuration overrides (internal helper)
fn apply_overrides(config: &mut GlobalConfig, overrides: ConfigOverrides) {
    // Apply network override (changes default URLs if needed)
    if let Some(network) = overrides.network {
        if config.bitcoin.network != network {
            config.bitcoin.network = network;
            // Update esplora URL to match network if not explicitly overridden
            if overrides.esplora_url.is_none() {
                config.bitcoin.esplora_url = match network {
                    NetworkType::Mainnet => "https://mempool.space/api".to_string(),
                    NetworkType::Testnet => "https://mempool.space/testnet/api".to_string(),
                    NetworkType::Signet => "https://mempool.space/signet/api".to_string(),
                    NetworkType::Regtest => "http://localhost:3002".to_string(),
                };
            }
        }
    }

    // Apply f1r3node overrides
    if let Some(host) = overrides.f1r3node_host {
        config.f1r3node.host = host;
    }
    if let Some(port) = overrides.f1r3node_grpc_port {
        config.f1r3node.grpc_port = port;
    }
    if let Some(port) = overrides.f1r3node_http_port {
        config.f1r3node.http_port = port;
    }

    // Apply esplora URL override (highest priority)
    if let Some(url) = overrides.esplora_url {
        config.bitcoin.esplora_url = url;
    }

    // Apply wallets directory override
    if let Some(wallets_dir) = overrides.wallets_dir {
        config.wallets_dir = Some(wallets_dir);
    }
}
