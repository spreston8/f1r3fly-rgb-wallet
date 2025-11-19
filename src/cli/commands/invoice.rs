//! Invoice CLI commands
//!
//! Handles RGB invoice generation and parsing via CLI.

use crate::config::{load_config, ConfigOverrides, NetworkType};
use crate::f1r3fly;
use crate::manager::WalletManager;

/// Error type for invoice command operations
#[derive(Debug, thiserror::Error)]
pub enum InvoiceCommandError {
    /// Manager error
    #[error("Manager error: {0}")]
    Manager(#[from] crate::manager::ManagerError),

    /// Config error
    #[error("Config error: {0}")]
    Config(#[from] crate::config::ConfigError),

    /// Invoice error
    #[error("Invoice error: {0}")]
    Invoice(#[from] crate::f1r3fly::InvoiceError),

    /// Bitcoin wallet error
    #[error("Bitcoin wallet error: {0}")]
    BitcoinWallet(#[from] crate::bitcoin::BitcoinWalletError),
}

/// Generate RGB invoice for receiving assets
///
/// Creates an RGB invoice that can be shared with sender.
pub async fn generate_invoice_cmd(
    wallet_name: Option<String>,
    contract_id: String,
    amount: u64,
    address: Option<String>,
    password: String,
    overrides: ConfigOverrides,
) -> Result<(), InvoiceCommandError> {
    // Load config
    let config = load_config(None, overrides)?;

    // Create manager
    let mut manager = WalletManager::new(config)?;

    // Determine wallet name
    let wallet_name = wallet_name.unwrap_or_else(|| "default".to_string());

    // Load wallet
    manager.load_wallet(&wallet_name, &password)?;

    // Get bitcoin wallet
    let bitcoin_wallet = manager
        .bitcoin_wallet_mut()
        .ok_or(crate::manager::ManagerError::WalletNotLoaded)?;

    // Generate invoice
    let generated = f1r3fly::generate_invoice(bitcoin_wallet, &contract_id, amount, address)?;

    // Persist wallet changes (address index incremented)
    bitcoin_wallet.persist()?;

    // Display invoice
    println!("\n✅ RGB Invoice Generated");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("\n📄 Invoice String:");
    println!("{}", generated.invoice.to_string());
    println!("\n📊 Invoice Details:");
    println!("  Contract ID:  {}", contract_id);
    println!("  Amount:       {}", amount);
    println!("  Address:      {}", generated.address);
    println!("  Seal:         {:?}", generated.seal);
    println!("\n💡 Share the invoice string with the sender to receive assets.");
    println!();

    Ok(())
}

/// Parse RGB invoice string and display details
///
/// Decodes an RGB invoice and shows its contents.
pub async fn parse_invoice_cmd(
    invoice_str: String,
    network: Option<NetworkType>,
) -> Result<(), InvoiceCommandError> {
    // Parse invoice
    let parsed = f1r3fly::parse_invoice(&invoice_str)?;

    // Extract address if network provided
    let address_info = if let Some(net) = network {
        let btc_network = net.to_bitcoin_network();
        match f1r3fly::get_address_from_invoice(&parsed.beneficiary, btc_network) {
            Ok(addr) => format!("\n  Address:      {}", addr),
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    // Display parsed invoice
    println!("\n✅ Invoice Parsed Successfully");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("\n📊 Invoice Details:");
    println!("  Contract ID:  {}", parsed.contract_id);
    if let Some(amt) = parsed.amount {
        println!("  Amount:       {}", amt);
    } else {
        println!("  Amount:       (not specified)");
    }
    print!("{}", address_info);
    println!("\n  Beneficiary:  {:?}", parsed.beneficiary);
    println!();

    Ok(())
}
