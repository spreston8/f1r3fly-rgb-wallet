//! Config command implementations

use crate::config::{ConfigError, GlobalConfig, NetworkType};

/// Initialize configuration file with network-specific defaults
pub fn init(network: Option<String>) -> Result<(), ConfigError> {
    // Parse network or default to regtest
    let network_type = match network.as_deref() {
        Some("regtest") | None => NetworkType::Regtest,
        Some("signet") => NetworkType::Signet,
        Some("testnet") => NetworkType::Testnet,
        Some("mainnet") => NetworkType::Mainnet,
        Some(n) => {
            return Err(ConfigError::InvalidNetwork(n.to_string()));
        }
    };

    // Create config with appropriate defaults
    let config = match network_type {
        NetworkType::Regtest => GlobalConfig::default_regtest(),
        NetworkType::Signet => GlobalConfig::default_signet(),
        NetworkType::Testnet => GlobalConfig::default_testnet(),
        NetworkType::Mainnet => GlobalConfig::default_mainnet(),
    };

    // Save to default location
    crate::config::save_config(&config, None)?;

    let config_path = crate::config::default_config_path()?;
    println!("✓ Configuration initialized for {:?}", network_type);
    println!("  Config file: {}", config_path.display());

    Ok(())
}

