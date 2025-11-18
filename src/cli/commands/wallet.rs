//! Wallet command implementations

use crate::config::{load_config, ConfigError, ConfigOverrides, NetworkType};
use crate::manager::WalletManager;
use crate::storage::file_system::{list_wallets as list_wallets_from_fs, FileSystemError};
use crate::storage::keys::KeyError;
use crate::storage::models::WalletKeys;
use bip39::Mnemonic;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum WalletCommandError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Manager error: {0}")]
    Manager(#[from] crate::manager::ManagerError),

    #[error("File system error: {0}")]
    FileSystem(#[from] FileSystemError),

    #[error("Key error: {0}")]
    Key(#[from] KeyError),

    #[error("Wallet '{0}' already exists")]
    WalletExists(String),

    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),
}

/// Create a new wallet with a generated mnemonic
pub fn create(name: String, password: String, overrides: ConfigOverrides) -> Result<(), WalletCommandError> {
    // Load config
    let config = load_config(None, overrides)?;
    let network = config.bitcoin.network;

    // Create manager and wallet using proper BDK initialization
    let mut manager = WalletManager::new(config)?;
    let mnemonic_str = manager.create_wallet(&name, &password)?;

    // Get the first address from the properly initialized wallet
    let first_address = manager.get_new_address()?;

    // Get wallet keys for F1r3fly public key display
    let mnemonic = Mnemonic::from_str(&mnemonic_str)
        .map_err(|e| WalletCommandError::InvalidMnemonic(e.to_string()))?;
    let keys = WalletKeys::from_mnemonic(&mnemonic, network)?;
    
    println!("✓ Wallet '{}' created successfully", name);
    println!();
    println!("  Network:           {:?}", network);
    println!("  First Address:     {}", first_address);
    println!("  F1r3fly Public Key: {}", keys.f1r3fly_public_key);
    println!();
    println!("  IMPORTANT: Write down your recovery phrase:");
    println!("  {}", mnemonic_str);
    println!();
    println!("  Keep this phrase safe and secret!");

    Ok(())
}

/// Import an existing wallet from a mnemonic phrase
pub fn import(
    name: String,
    mnemonic_str: String,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), WalletCommandError> {
    // Load config
    let config = load_config(None, overrides)?;
    let network = config.bitcoin.network;

    // Create manager and import wallet using proper BDK initialization
    let mut manager = WalletManager::new(config)?;
    manager.import_wallet(&name, &mnemonic_str, &password)?;

    // Get the first address from the properly initialized wallet
    let first_address = manager.get_new_address()?;

    // Get wallet keys for F1r3fly public key display
    let mnemonic = Mnemonic::from_str(&mnemonic_str)
        .map_err(|e| WalletCommandError::InvalidMnemonic(e.to_string()))?;
    let keys = WalletKeys::from_mnemonic(&mnemonic, network)?;
    
    println!("✓ Wallet '{}' imported successfully", name);
    println!();
    println!("  Network:           {:?}", network);
    println!("  First Address:     {}", first_address);
    println!("  F1r3fly Public Key: {}", keys.f1r3fly_public_key);

    Ok(())
}

/// List all wallets
pub fn list(overrides: ConfigOverrides) -> Result<(), WalletCommandError> {
    // Load config to get wallets directory
    let config = load_config(None, overrides)?;
    let custom_base = config.wallets_dir.as_deref();
    
    let wallets = list_wallets_from_fs(custom_base)?;

    if wallets.is_empty() {
        println!("No wallets found.");
        println!();
        println!("Create a new wallet with:");
        println!("  f1r3fly-rgb-wallet wallet create <name> --password <password>");
        return Ok(());
    }

    println!("Wallets ({}):", wallets.len());
    println!();

    for wallet in wallets {
        println!("  {} [{}]", wallet.name, format_network(wallet.network));
        println!("    Created: {}", wallet.created_at.format("%Y-%m-%d %H:%M:%S"));
        if let Some(last_sync) = wallet.last_sync {
            println!("    Last Sync: {}", last_sync.format("%Y-%m-%d %H:%M:%S"));
        }
        println!();
    }

    Ok(())
}

fn format_network(network: NetworkType) -> &'static str {
    match network {
        NetworkType::Regtest => "Regtest",
        NetworkType::Signet => "Signet",
        NetworkType::Testnet => "Testnet",
        NetworkType::Mainnet => "Mainnet",
    }
}

