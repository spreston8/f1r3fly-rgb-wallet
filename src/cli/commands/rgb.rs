//! RGB Asset CLI Commands
//!
//! Command implementations for RGB asset operations including issuance,
//! balance queries, and asset information retrieval.

use crate::config::{load_config, ConfigOverrides};
use crate::f1r3fly::{AssetBalance, AssetInfo, AssetListItem, IssueAssetRequest};
use crate::manager::WalletManager;

/// Error type for RGB command operations
#[derive(Debug, thiserror::Error)]
pub enum RgbCommandError {
    /// Manager error
    #[error("Manager error: {0}")]
    Manager(#[from] crate::manager::ManagerError),

    /// Config error
    #[error("Config error: {0}")]
    Config(#[from] crate::config::ConfigError),
}

/// Issue a new RGB asset
///
/// Creates a new fungible token using the specified genesis UTXO.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `ticker` - Asset ticker (e.g., "BTC")
/// * `name` - Asset full name (e.g., "Bitcoin")
/// * `supply` - Total supply
/// * `precision` - Decimal precision (e.g., 8 for Bitcoin)
/// * `genesis_utxo` - Genesis UTXO in format "txid:vout"
/// * `password` - Wallet password
/// * `overrides` - Config overrides from CLI/env
///
/// # Returns
///
/// `AssetInfo` with the newly created asset details
///
/// # Errors
///
/// Returns error if wallet not found, UTXO invalid, or issuance fails
///
/// # Example
///
/// ```ignore
/// issue_asset(
///     "my-wallet",
///     "USD",
///     "US Dollar",
///     100000000,
///     2,
///     "abc123...def:0",
///     "password123",
///     &overrides,
/// ).await?;
/// ```
pub async fn issue_asset(
    wallet_name: &str,
    ticker: &str,
    name: &str,
    supply: u64,
    precision: u8,
    genesis_utxo: &str,
    password: &str,
    overrides: &ConfigOverrides,
) -> Result<AssetInfo, RgbCommandError> {
    // Load config
    let config = load_config(None, overrides.clone())?;

    // Create manager
    let mut manager = WalletManager::new(config)?;

    // Load wallet
    manager.load_wallet(wallet_name, password)?;

    // Create issuance request
    let request = IssueAssetRequest {
        ticker: ticker.to_string(),
        name: name.to_string(),
        supply,
        precision,
        genesis_utxo: genesis_utxo.to_string(),
    };

    // Issue asset
    let asset_info = manager.issue_asset(request).await?;

    // Display result
    println!("✅ Asset issued successfully!");
    println!();
    println!("Contract ID:   {}", asset_info.contract_id);
    println!("Ticker:        {}", asset_info.ticker);
    println!("Name:          {}", asset_info.name);
    println!("Total Supply:  {}", asset_info.supply);
    println!("Precision:     {}", asset_info.precision);
    println!("Genesis Seal:  {}", asset_info.genesis_seal);
    println!("Registry URI:  {}", asset_info.registry_uri);

    Ok(asset_info)
}

/// List all RGB assets in the wallet
///
/// Displays all issued assets with their metadata.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `password` - Wallet password
/// * `overrides` - Config overrides from CLI/env
///
/// # Returns
///
/// Vector of `AssetListItem`
///
/// # Errors
///
/// Returns error if wallet not found or query fails
///
/// # Example
///
/// ```ignore
/// list_assets("my-wallet", "password123", &overrides)?;
/// ```
pub fn list_assets(
    wallet_name: &str,
    password: &str,
    overrides: &ConfigOverrides,
) -> Result<Vec<AssetListItem>, RgbCommandError> {
    // Load config
    let config = load_config(None, overrides.clone())?;

    // Create manager
    let mut manager = WalletManager::new(config)?;

    // Load wallet
    manager.load_wallet(wallet_name, password)?;

    // List assets
    let assets = manager.list_assets()?;

    // Display results
    if assets.is_empty() {
        println!("No RGB assets found in this wallet.");
    } else {
        println!("RGB Assets ({}):", assets.len());
        println!();
        for asset in &assets {
            println!("Contract ID: {}", asset.contract_id);
            println!("  Ticker:      {}", asset.ticker);
            println!("  Name:        {}", asset.name);
            println!("  Registry:    {}", asset.registry_uri);
            println!();
        }
    }

    Ok(assets)
}

/// Get RGB balance for all assets or a specific asset
///
/// Queries F1r3node contract state and displays per-asset and per-UTXO balances.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `contract_id` - Optional contract ID (if None, shows all assets)
/// * `password` - Wallet password
/// * `overrides` - Config overrides from CLI/env
///
/// # Returns
///
/// Vector of `AssetBalance` (one per asset)
///
/// # Errors
///
/// Returns error if wallet not found or balance query fails
///
/// # Example
///
/// ```ignore
/// // Get all balances
/// rgb_balance("my-wallet", None, "password123", &overrides).await?;
///
/// // Get specific asset balance
/// rgb_balance("my-wallet", Some("contract_id_123"), "password123", &overrides).await?;
/// ```
pub async fn rgb_balance(
    wallet_name: &str,
    contract_id: Option<&str>,
    password: &str,
    overrides: &ConfigOverrides,
) -> Result<Vec<AssetBalance>, RgbCommandError> {
    // Load config
    let config = load_config(None, overrides.clone())?;

    // Create manager
    let mut manager = WalletManager::new(config)?;

    // Load wallet
    manager.load_wallet(wallet_name, password)?;

    // Get balances
    let balances = if let Some(cid) = contract_id {
        // Specific asset
        let balance = manager.get_asset_balance(cid).await?;
        vec![balance]
    } else {
        // All assets
        manager.get_rgb_balance().await?
    };

    // Display results
    if balances.is_empty() {
        println!("No RGB asset balances found.");
    } else {
        println!("RGB Balances:");
        println!();
        for balance in &balances {
            println!("Asset: {} ({})", balance.name, balance.ticker);
            println!("  Contract ID: {}", balance.contract_id);
            println!("  Total:       {}", format_amount(balance.total, 8)); // Default to 8 decimals for display
            
            if !balance.utxo_balances.is_empty() {
                println!("  UTXOs:");
                for utxo_balance in &balance.utxo_balances {
                    println!(
                        "    {} - {}",
                        utxo_balance.outpoint,
                        format_amount(utxo_balance.amount, 8)
                    );
                }
            }
            println!();
        }
    }

    Ok(balances)
}

/// Get detailed information about a specific RGB asset
///
/// Retrieves full metadata and genesis information for a contract.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `contract_id` - Contract ID
/// * `password` - Wallet password
/// * `overrides` - Config overrides from CLI/env
///
/// # Returns
///
/// `AssetInfo` with full asset details
///
/// # Errors
///
/// Returns error if wallet not found, contract not found, or query fails
///
/// # Example
///
/// ```ignore
/// get_contract_info("my-wallet", "contract_id_123", "password123", &overrides)?;
/// ```
pub fn get_contract_info(
    wallet_name: &str,
    contract_id: &str,
    password: &str,
    overrides: &ConfigOverrides,
) -> Result<AssetInfo, RgbCommandError> {
    // Load config
    let config = load_config(None, overrides.clone())?;

    // Create manager
    let mut manager = WalletManager::new(config)?;

    // Load wallet
    manager.load_wallet(wallet_name, password)?;

    // Get asset info
    let asset_info = manager.get_asset_info(contract_id)?;

    // Display result
    println!("RGB Asset Information:");
    println!();
    println!("Contract ID:   {}", asset_info.contract_id);
    println!("Ticker:        {}", asset_info.ticker);
    println!("Name:          {}", asset_info.name);
    println!("Total Supply:  {}", asset_info.supply);
    println!("Precision:     {}", asset_info.precision);
    println!("Genesis Seal:  {}", asset_info.genesis_seal);
    println!("Registry URI:  {}", asset_info.registry_uri);

    Ok(asset_info)
}

/// Format amount with decimal precision for display
///
/// Converts raw integer amount to decimal string representation.
///
/// # Arguments
///
/// * `amount` - Raw integer amount
/// * `precision` - Number of decimal places
///
/// # Returns
///
/// Formatted string (e.g., "1.23456789")
///
/// # Example
///
/// ```ignore
/// let formatted = format_amount(123456789, 8);
/// assert_eq!(formatted, "1.23456789");
/// ```
fn format_amount(amount: u64, precision: u8) -> String {
    if precision == 0 {
        return amount.to_string();
    }

    let divisor = 10u64.pow(precision as u32);
    let integer_part = amount / divisor;
    let fractional_part = amount % divisor;

    if fractional_part == 0 {
        format!("{}", integer_part)
    } else {
        // Format with leading zeros if needed
        format!(
            "{}.{:0width$}",
            integer_part,
            fractional_part,
            width = precision as usize
        )
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
    }
}

