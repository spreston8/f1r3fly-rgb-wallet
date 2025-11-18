//! Bitcoin command implementations

use crate::config::{load_config, ConfigError, ConfigOverrides};
use crate::manager::{ManagerError, WalletManager};
use crate::bitcoin::FeeRateConfig;

#[derive(Debug, thiserror::Error)]
pub enum BitcoinCommandError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Manager error: {0}")]
    Manager(#[from] ManagerError),

    #[error("Wallet not specified. Use --wallet <name>")]
    WalletNotSpecified,
}

/// Sync wallet with blockchain
pub fn sync(wallet_name: Option<String>, password: String, overrides: ConfigOverrides) -> Result<(), BitcoinCommandError> {
    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;
    
    // Load config
    let config = load_config(None, overrides)?;
    
    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;
    
    // Sync wallet
    let result = manager.sync_wallet()?;
    
    println!("✓ Wallet synced successfully");
    println!("  Height: {}", result.height);
    println!("  New transactions: {}", result.new_txs);
    println!("  Updated transactions: {}", result.updated_txs);
    
    Ok(())
}

/// Get Bitcoin balance
pub fn get_balance(wallet_name: Option<String>, password: String, overrides: ConfigOverrides) -> Result<(), BitcoinCommandError> {
    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;
    
    // Load config
    let config = load_config(None, overrides)?;
    
    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;
    
    // Get balance
    let balance = manager.get_balance()?;
    
    println!("Bitcoin Balance:");
    println!("  Confirmed:   {} sats ({:.8} BTC)", balance.confirmed, balance.confirmed as f64 / 100_000_000.0);
    println!("  Unconfirmed: {} sats ({:.8} BTC)", balance.unconfirmed, balance.unconfirmed as f64 / 100_000_000.0);
    println!("  Total:       {} sats ({:.8} BTC)", balance.total, balance.total as f64 / 100_000_000.0);
    
    // Display RGB-occupied UTXO warning
    let rgb_count = manager.rgb_occupied().len();
    if rgb_count > 0 {
        println!();
        println!("⚠️  RGB Assets: {} UTXO(s) are RGB-occupied and protected", rgb_count);
        println!("   Use 'rgb-balance' command to view RGB token holdings");
    }
    
    Ok(())
}

/// Get wallet addresses
pub fn get_addresses(wallet_name: Option<String>, count: usize, password: String, overrides: ConfigOverrides) -> Result<(), BitcoinCommandError> {
    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;
    
    // Load config
    let config = load_config(None, overrides)?;
    
    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;
    
    // Get addresses
    let addresses = manager.get_addresses(Some(count as u32))?;
    
    println!("Wallet Addresses:");
    println!();
    
    for (i, addr_info) in addresses.iter().enumerate() {
        let used = if addr_info.is_used { "used" } else { "unused" };
        println!("  {}: {} ({}, index: {})", 
            i + 1, 
            addr_info.address, 
            used,
            addr_info.index
        );
    }
    
    let display_count = addresses.len();
    
    if addresses.len() > display_count {
        println!();
        println!("  ... and {} more addresses", addresses.len() - display_count);
    }
    
    Ok(())
}

/// Create a UTXO via self-send
pub fn create_utxo(
    wallet_name: Option<String>,
    amount_btc: f64,
    fee_rate: Option<f32>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;
    
    // Load config
    let config = load_config(None, overrides)?;
    
    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;
    
    // Display RGB warning if applicable
    let rgb_count = manager.rgb_occupied().len();
    if rgb_count > 0 {
        println!("⚠️  Notice: {} RGB-occupied UTXO(s) in wallet (will be protected from spending)", rgb_count);
        println!();
    }
    
    // Convert BTC to sats
    let amount_sats = (amount_btc * 100_000_000.0) as u64;
    
    // Create UTXO
    let fee_config = fee_rate
        .and_then(|rate| FeeRateConfig::new(rate as f64).ok())
        .unwrap_or(FeeRateConfig::medium_priority());
    let result = manager.create_utxo(amount_sats, &fee_config, false)?;
    
    println!("✓ UTXO created successfully");
    println!("  Transaction ID: {}", result.txid);
    println!("  Output: {}:{}", result.outpoint.txid, result.outpoint.vout);
    println!("  Amount: {} sats ({:.8} BTC)", result.amount, result.amount as f64 / 100_000_000.0);
    println!("  Fee: {} sats", result.fee);
    println!("  Fee rate: {:.2} sat/vB", result.fee_rate);
    
    Ok(())
}

/// Send Bitcoin to an address
pub fn send_bitcoin(
    wallet_name: Option<String>,
    to_address: String,
    amount_sats: u64,
    fee_rate: Option<f32>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;
    
    // Load config
    let config = load_config(None, overrides)?;
    
    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;
    
    // Display RGB warning if applicable
    let rgb_count = manager.rgb_occupied().len();
    if rgb_count > 0 {
        println!("⚠️  SAFETY: {} RGB-occupied UTXO(s) detected - these will NOT be spent", rgb_count);
        println!("   RGB assets are safe. Only regular Bitcoin UTXOs will be used.");
        println!();
    }
    
    // Send Bitcoin
    let fee_config = fee_rate
        .and_then(|rate| FeeRateConfig::new(rate as f64).ok())
        .unwrap_or(FeeRateConfig::medium_priority());
    let txid = manager.send_bitcoin(&to_address, amount_sats, &fee_config)?;
    
    println!("✓ Bitcoin sent successfully");
    println!("  Transaction ID: {}", txid);
    println!("  Recipient: {}", to_address);
    println!("  Amount: {} sats ({:.8} BTC)", amount_sats, amount_sats as f64 / 100_000_000.0);
    
    Ok(())
}

