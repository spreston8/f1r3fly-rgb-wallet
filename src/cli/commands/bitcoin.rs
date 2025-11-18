//! Bitcoin command implementations

use crate::bitcoin::FeeRateConfig;
use crate::config::{load_config, ConfigError, ConfigOverrides};
use crate::manager::{ManagerError, WalletManager};

#[derive(Debug, thiserror::Error)]
pub enum BitcoinCommandError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Manager error: {0}")]
    Manager(#[from] ManagerError),

    #[error("Wallet not specified. Use --wallet <name>")]
    WalletNotSpecified,

    #[error("Invalid output format: {0}")]
    InvalidFormat(String),
}

/// Sync wallet with blockchain
pub fn sync(
    wallet_name: Option<String>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
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
pub async fn get_balance(
    wallet_name: Option<String>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
    use crate::types::{UtxoFilter, UtxoStatus};

    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;

    // Load config
    let config = load_config(None, overrides)?;

    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;

    // Get balance
    let balance = manager.get_balance()?;

    println!("Bitcoin Balance:");
    println!(
        "  Confirmed:   {} sats ({:.8} BTC)",
        balance.confirmed,
        balance.confirmed as f64 / 100_000_000.0
    );
    println!(
        "  Unconfirmed: {} sats ({:.8} BTC)",
        balance.unconfirmed,
        balance.unconfirmed as f64 / 100_000_000.0
    );
    println!(
        "  Total:       {} sats ({:.8} BTC)",
        balance.total,
        balance.total as f64 / 100_000_000.0
    );

    // Get UTXO summary
    let all_utxos = manager.list_utxos(UtxoFilter::default()).await?;
    let available = all_utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Available)
        .count();
    let rgb_occupied = all_utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::RgbOccupied)
        .count();
    let unconfirmed = all_utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Unconfirmed)
        .count();

    // Display UTXO summary
    println!();
    println!("UTXO Summary:");
    println!("  Total UTXOs:    {}", all_utxos.len());
    println!("  Available:      {}", available);
    if rgb_occupied > 0 {
        println!("  RGB-Occupied:   {} ⚠️  (protected)", rgb_occupied);
    } else {
        println!("  RGB-Occupied:   {}", rgb_occupied);
    }
    if unconfirmed > 0 {
        println!("  Unconfirmed:    {}", unconfirmed);
    }

    // Helpful hints
    if rgb_occupied > 0 || all_utxos.len() > 0 {
        println!();
        if rgb_occupied > 0 {
            println!("💡 Use 'rgb-balance' to view RGB token holdings");
        }
        if all_utxos.len() > 0 {
            println!("💡 Use 'list-utxos' for detailed UTXO information");
        }
    }

    Ok(())
}

/// Get wallet addresses
pub fn get_addresses(
    wallet_name: Option<String>,
    count: usize,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
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
        println!(
            "  {}: {} ({}, index: {})",
            i + 1,
            addr_info.address,
            used,
            addr_info.index
        );
    }

    let display_count = addresses.len();

    if addresses.len() > display_count {
        println!();
        println!(
            "  ... and {} more addresses",
            addresses.len() - display_count
        );
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
        println!(
            "⚠️  Notice: {} RGB-occupied UTXO(s) in wallet (will be protected from spending)",
            rgb_count
        );
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
    println!(
        "  Output: {}:{}",
        result.outpoint.txid, result.outpoint.vout
    );
    println!(
        "  Amount: {} sats ({:.8} BTC)",
        result.amount,
        result.amount as f64 / 100_000_000.0
    );
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
        println!(
            "⚠️  SAFETY: {} RGB-occupied UTXO(s) detected - these will NOT be spent",
            rgb_count
        );
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
    println!(
        "  Amount: {} sats ({:.8} BTC)",
        amount_sats,
        amount_sats as f64 / 100_000_000.0
    );

    Ok(())
}

/// List all UTXOs with filtering and formatting options
pub async fn list_utxos(
    wallet_name: Option<String>,
    password: String,
    available_only: bool,
    rgb_only: bool,
    confirmed_only: bool,
    min_amount_btc: Option<f64>,
    format_str: String,
    overrides: ConfigOverrides,
) -> Result<(), BitcoinCommandError> {
    use crate::types::{OutputFormat, UtxoFilter};

    let wallet_name = wallet_name.ok_or(BitcoinCommandError::WalletNotSpecified)?;

    // Parse output format
    let format = format_str
        .parse::<OutputFormat>()
        .map_err(|e| BitcoinCommandError::InvalidFormat(e))?;

    // Convert min_amount from BTC to sats
    let min_amount_sats = min_amount_btc.map(|btc| (btc * 100_000_000.0) as u64);

    // Build filter
    let filter = UtxoFilter {
        available_only,
        rgb_only,
        confirmed_only,
        min_amount_sats,
    };

    // Load config and wallet
    let config = load_config(None, overrides)?;
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;

    // Get UTXOs
    let utxos = manager.list_utxos(filter).await?;

    // Display based on format
    match format {
        OutputFormat::Table => print_utxos_table(&wallet_name, &utxos),
        OutputFormat::Json => print_utxos_json(&wallet_name, &utxos),
        OutputFormat::Compact => print_utxos_compact(&utxos),
    }

    Ok(())
}

/// Format outpoint for table display (truncate long txids)
fn format_outpoint(outpoint: &str) -> String {
    if outpoint.len() > 22 {
        let parts: Vec<&str> = outpoint.split(':').collect();
        if parts.len() == 2 {
            let txid = parts[0];
            let vout = parts[1];
            if txid.len() > 15 {
                return format!("{}...{}:{}", &txid[..6], &txid[txid.len() - 3..], vout);
            }
        }
    }
    outpoint.to_string()
}

/// Format RGB assets for display
fn format_rgb_assets(assets: &[crate::types::RgbSealInfo]) -> String {
    if assets.is_empty() {
        return "-".to_string();
    }

    assets
        .iter()
        .map(|a| {
            if let Some(amount) = a.amount {
                format!("{} ({})", a.ticker, amount)
            } else {
                format!("{} (change)", a.ticker)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Print summary statistics
fn print_summary(utxos: &[crate::types::UtxoInfo]) {
    use crate::types::UtxoStatus;

    let total = utxos.len();
    let available = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Available)
        .count();
    let rgb_occupied = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::RgbOccupied)
        .count();
    let unconfirmed = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Unconfirmed)
        .count();

    let total_available_btc: f64 = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Available)
        .map(|u| u.amount_btc)
        .sum();

    let total_rgb_btc: f64 = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::RgbOccupied)
        .map(|u| u.amount_btc)
        .sum();

    println!("Total UTXOs: {}", total);
    println!("Available: {} ({:.8} BTC)", available, total_available_btc);
    println!("RGB-Occupied: {} ({:.8} BTC)", rgb_occupied, total_rgb_btc);
    if unconfirmed > 0 {
        println!("Unconfirmed: {}", unconfirmed);
    }
}

/// Print UTXOs in table format
fn print_utxos_table(wallet_name: &str, utxos: &[crate::types::UtxoInfo]) {
    println!("UTXO List for wallet: {}", wallet_name);
    println!("=========================================");
    println!();

    if utxos.is_empty() {
        println!("No UTXOs found.");
        return;
    }

    // Header
    println!(
        "{:<22} | {:<12} | {:<13} | {:<13} | {}",
        "Outpoint", "Amount (BTC)", "Confirmations", "Status", "RGB Assets"
    );
    println!(
        "{:-<22}-+-{:-<12}-+-{:-<13}-+-{:-<13}-+-{:-<20}",
        "", "", "", "", ""
    );

    // Rows
    for utxo in utxos {
        let outpoint_short = format_outpoint(&utxo.outpoint);
        let rgb_display = format_rgb_assets(&utxo.rgb_assets);

        println!(
            "{:<22} | {:>12.8} | {:>13} | {:<13} | {}",
            outpoint_short, utxo.amount_btc, utxo.confirmations, utxo.status, rgb_display
        );
    }

    println!();
    print_summary(utxos);
}

/// Print UTXOs in JSON format
fn print_utxos_json(wallet_name: &str, utxos: &[crate::types::UtxoInfo]) {
    use crate::types::UtxoStatus;
    use serde_json::json;

    let available = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Available)
        .count();
    let rgb_occupied = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::RgbOccupied)
        .count();

    let total_available_btc: f64 = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::Available)
        .map(|u| u.amount_btc)
        .sum();

    let total_rgb_btc: f64 = utxos
        .iter()
        .filter(|u| u.status == UtxoStatus::RgbOccupied)
        .map(|u| u.amount_btc)
        .sum();

    let output = json!({
        "wallet": wallet_name,
        "total_utxos": utxos.len(),
        "available_count": available,
        "rgb_occupied_count": rgb_occupied,
        "total_available_btc": total_available_btc,
        "total_rgb_occupied_btc": total_rgb_btc,
        "utxos": utxos
    });

    match serde_json::to_string_pretty(&output) {
        Ok(json_str) => println!("{}", json_str),
        Err(e) => eprintln!("Error serializing to JSON: {}", e),
    }
}

/// Print UTXOs in compact format (script-friendly)
fn print_utxos_compact(utxos: &[crate::types::UtxoInfo]) {
    use crate::types::UtxoStatus;

    for utxo in utxos {
        let status_str = match utxo.status {
            UtxoStatus::Available => "available",
            UtxoStatus::RgbOccupied => "rgb-occupied",
            UtxoStatus::Unconfirmed => "unconfirmed",
        };

        let rgb_info = if utxo.rgb_assets.is_empty() {
            String::new()
        } else {
            let assets_str = utxo
                .rgb_assets
                .iter()
                .map(|a| {
                    if let Some(amount) = a.amount {
                        format!("{}:{}", a.ticker, amount)
                    } else {
                        format!("{}:change", a.ticker)
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(" {}", assets_str)
        };

        println!(
            "{} {:.8} {}{}",
            utxo.outpoint, utxo.amount_btc, status_str, rgb_info
        );
    }
}
