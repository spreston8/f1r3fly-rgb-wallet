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
pub fn create(
    name: String,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), WalletCommandError> {
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
        println!(
            "    Created: {}",
            wallet.created_at.format("%Y-%m-%d %H:%M:%S")
        );
        if let Some(last_sync) = wallet.last_sync {
            println!("    Last Sync: {}", last_sync.format("%Y-%m-%d %H:%M:%S"));
        }
        println!();
    }

    Ok(())
}

/// Get F1r3fly public key
///
/// Returns the public key from the F1r3fly executor (master key).
/// This is the key used for all RGB transfer and claim signatures.
/// No password required since it's public information meant to be shared.
pub fn get_f1r3fly_pubkey(overrides: ConfigOverrides) -> Result<(), WalletCommandError> {
    // Load config
    let config = load_config(None, overrides)?;

    // IMPORTANT: Get the public key from the F1r3flyExecutor (using config master key)
    // This is the ACTUAL key used for signing claims and transfers.
    // The wallet-specific F1r3fly key stored in keys.json is reserved for future use.
    //
    // Create executor using the config's master key (same as what's used for actual operations)
    use f1r3fly_rgb::F1r3flyExecutor;
    use node_cli::connection_manager::{ConnectionConfig, F1r3flyConnectionManager};

    let connection_config = ConnectionConfig::new(
        config.f1r3node.host.clone(),
        config.f1r3node.grpc_port,
        config.f1r3node.http_port,
        config.f1r3node.master_key.clone(),
    );

    let connection = F1r3flyConnectionManager::new(connection_config);
    let executor = F1r3flyExecutor::with_connection(connection);

    // Get the public key from the executor (at derivation index 0, which is the default)
    let pubkey = executor
        .get_public_key()
        .expect("Failed to get public key from executor");

    // Use uncompressed format (matches invoice generation)
    let pubkey_hex = hex::encode(pubkey.serialize_uncompressed());

    // Display public key (not encrypted, safe to show)
    println!("F1r3fly Public Key:");
    println!("  {}", pubkey_hex);
    println!();
    println!("💡 Share this public key with senders who want to transfer RGB assets to you.");

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
