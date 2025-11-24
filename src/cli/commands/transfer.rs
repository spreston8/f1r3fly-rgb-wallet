//! RGB transfer command implementations

use crate::bitcoin::FeeRateConfig;
use crate::config::{load_config, ConfigError, ConfigOverrides};
use crate::manager::{ManagerError, WalletManager};
use crate::storage::{ClaimStatus, PendingClaim};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum TransferCommandError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Manager error: {0}")]
    Manager(#[from] ManagerError),

    #[error("Wallet not specified. Use --wallet <name>")]
    WalletNotSpecified,

    #[error("Invalid output format: {0}")]
    InvalidFormat(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Send RGB asset transfer using an invoice
pub async fn send_transfer(
    wallet_name: Option<String>,
    invoice: String,
    recipient_pubkey: String,
    fee_rate: Option<f32>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), TransferCommandError> {
    let wallet_name = wallet_name.ok_or(TransferCommandError::WalletNotSpecified)?;

    // Load config
    let config = load_config(None, overrides)?;

    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;

    // Determine fee rate
    let fee_rate_config = match fee_rate {
        Some(rate) => FeeRateConfig::new(rate as f64)
            .map_err(|e| TransferCommandError::Manager(ManagerError::Utxo(e)))?,
        None => FeeRateConfig::medium_priority(),
    };

    println!("📤 Sending RGB transfer...");
    println!("  Invoice: {}...", &invoice[..invoice.len().min(50)]);
    println!(
        "  Recipient pubkey: {}...",
        &recipient_pubkey[..recipient_pubkey.len().min(16)]
    );
    println!();

    // Send transfer
    let response = manager
        .send_transfer(&invoice, recipient_pubkey, &fee_rate_config)
        .await?;

    println!("✓ Transfer sent successfully!");
    println!();
    println!("Transaction Details:");
    println!("  Bitcoin TX ID: {}", response.bitcoin_txid);
    println!("  Consignment:   {}", response.consignment_path.display());
    println!();
    println!("Transfer Summary:");
    println!("  Amount sent:   {}", response.amount);
    println!("  Change amount: {}", response.change_amount);
    println!();
    println!("📋 Next Steps:");
    println!("  1. Share consignment file with recipient:");
    println!("     {}", response.consignment_path.display());
    println!("  2. Recipient should accept consignment using:");
    println!("     accept-consignment --consignment-path <path>");

    Ok(())
}

/// Accept RGB consignment (transfer or genesis)
pub async fn accept_consignment(
    wallet_name: Option<String>,
    consignment_path: String,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), TransferCommandError> {
    let wallet_name = wallet_name.ok_or(TransferCommandError::WalletNotSpecified)?;

    // Load config
    let config = load_config(None, overrides)?;

    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;

    // Validate consignment file exists
    let consignment_file = Path::new(&consignment_path);
    if !consignment_file.exists() {
        return Err(TransferCommandError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Consignment file not found: {}", consignment_path),
        )));
    }

    println!("📥 Accepting consignment...");
    println!("  File: {}", consignment_path);
    println!();

    // Accept consignment
    let response = manager.accept_consignment(&consignment_path).await?;

    println!("✓ Consignment accepted successfully!");
    println!();
    println!("Contract Details:");
    println!("  Contract ID: {}", response.contract_id);
    println!("  Ticker:      {}", response.ticker);
    println!("  Name:        {}", response.name);
    println!("  Seals:       {} imported", response.seals_imported);
    println!();
    println!("📋 Next Steps:");
    println!("  1. Sync wallet to finalize claim:");
    println!("     sync --password <password>");
    println!("  2. Check balance:");
    println!("     rgb-balance --password <password>");
    println!("  3. View claim status:");
    println!("     list-claims --password <password>");

    Ok(())
}

/// List RGB claim history
pub async fn list_claims(
    wallet_name: Option<String>,
    contract_id: Option<String>,
    format: String,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), TransferCommandError> {
    let wallet_name = wallet_name.ok_or(TransferCommandError::WalletNotSpecified)?;

    // Load config
    let config = load_config(None, overrides)?;

    // Create manager and load wallet
    let mut manager = WalletManager::new(config)?;
    manager.load_wallet(&wallet_name, &password)?;

    // Get claims
    let claims = manager.list_claims(contract_id.as_deref())?;

    match format.as_str() {
        "table" => print_claims_table(&claims),
        "json" => print_claims_json(&claims)?,
        _ => {
            return Err(TransferCommandError::InvalidFormat(format));
        }
    }

    Ok(())
}

/// Print claims in table format
fn print_claims_table(claims: &[PendingClaim]) {
    if claims.is_empty() {
        println!("No claims found.");
        return;
    }

    println!("RGB Claims ({}):", claims.len());
    println!();
    println!(
        "{:<8} {:<45} {:<12} {:<20} {:<68}",
        "ID", "Witness ID", "Status", "Contract", "Actual UTXO"
    );
    println!("{:-<160}", "");

    for claim in claims {
        let status_str = match claim.status {
            ClaimStatus::Pending => "Pending",
            ClaimStatus::Claimed => "✓ Claimed",
            ClaimStatus::Failed => "✗ Failed",
        };

        let actual_utxo = if let (Some(txid), Some(vout)) = (&claim.actual_txid, claim.actual_vout)
        {
            format!("{}:{}", &txid[..16], vout)
        } else {
            "N/A".to_string()
        };

        let contract_short = if claim.contract_id.len() > 20 {
            format!("{}...", &claim.contract_id[..17])
        } else {
            claim.contract_id.clone()
        };

        println!(
            "{:<8} {:<45} {:<12} {:<20} {:<68}",
            claim.id.unwrap_or(0),
            &claim.witness_id[..claim.witness_id.len().min(45)],
            status_str,
            contract_short,
            actual_utxo
        );
    }

    println!();
    println!("Summary:");
    let pending = claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Pending)
        .count();
    let claimed = claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Claimed)
        .count();
    let failed = claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Failed)
        .count();
    println!("  Pending: {}", pending);
    println!("  Claimed: {}", claimed);
    println!("  Failed:  {}", failed);
}

/// Print claims in JSON format
fn print_claims_json(claims: &[PendingClaim]) -> Result<(), TransferCommandError> {
    let json = serde_json::to_string_pretty(&claims).map_err(|e| {
        TransferCommandError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("JSON serialization error: {}", e),
        ))
    })?;
    println!("{}", json);
    Ok(())
}
