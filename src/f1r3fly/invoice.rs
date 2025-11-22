//! RGB Invoice Generation - Wallet Wrapper
//!
//! Thin wrapper over f1r3fly-rgb core invoice functionality.
//! Handles wallet-specific concerns: address selection, Bitcoin integration, error conversion.
//!
//! All RGB protocol logic is delegated to the core library.

use bitcoin::Address;
use f1r3fly_rgb::{
    extract_seal, generate_invoice as core_generate, get_recipient_address,
    parse_invoice as core_parse, GeneratedInvoice, ParsedInvoice, RgbBeneficiary,
};
use hypersonic::ContractId;
use rgb::Consensus;
use std::str::FromStr;

use crate::bitcoin::BitcoinWallet;
use crate::config::NetworkType;
use crate::f1r3fly::F1r3flyContractsManager;

/// Wallet-specific invoice error type
#[derive(Debug, thiserror::Error)]
pub enum InvoiceError {
    /// Bitcoin wallet error
    #[error("Bitcoin wallet error: {0}")]
    BitcoinWallet(String),

    /// Core library error
    #[error("Invoice error: {0}")]
    Core(#[from] f1r3fly_rgb::F1r3flyRgbError),

    /// Invalid address
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    /// Invalid contract ID
    #[error("Invalid contract ID: {0}")]
    InvalidContractId(String),

    /// Invalid network
    #[error("Invalid network: {0}")]
    InvalidNetwork(String),
}

/// Generate RGB invoice for receiving assets
///
/// Thin wrapper that:
/// 1. Gets a Bitcoin address from the wallet (or uses provided address)
/// 2. Delegates to core library's `generate_invoice()`
/// 3. Returns the result
///
/// # Arguments
///
/// * `bitcoin_wallet` - Bitcoin wallet to get receiving address from
/// * `contract_id_str` - RGB contract ID as string
/// * `amount` - Amount to receive (in smallest unit)
/// * `address_override` - Optional specific address to use (for testing/advanced use)
///
/// # Returns
///
/// `GeneratedInvoice` with invoice string, seal, and metadata
pub fn generate_invoice(
    bitcoin_wallet: &mut BitcoinWallet,
    contract_id_str: &str,
    amount: u64,
    address_override: Option<String>,
) -> Result<GeneratedInvoice, InvoiceError> {
    // Parse contract ID
    let contract_id = ContractId::from_str(contract_id_str)
        .map_err(|e| InvoiceError::InvalidContractId(format!("{}", e)))?;

    // Get Bitcoin address (use override or get from wallet)
    let address = if let Some(addr_str) = address_override {
        Address::from_str(&addr_str)
            .map_err(|e| InvoiceError::InvalidAddress(format!("{}", e)))?
            .assume_checked()
    } else {
        // Get next unused address from wallet
        let addr_info = bitcoin_wallet
            .inner_mut()
            .reveal_next_address(bdk_wallet::KeychainKind::External);

        // CRITICAL: Persist wallet so address is saved to database
        // This ensures the address is included in future derivation_index() queries
        bitcoin_wallet.persist().map_err(|e| {
            InvoiceError::InvalidAddress(format!("Failed to persist wallet: {}", e))
        })?;

        addr_info.address
    };

    // Determine network parameters
    let network_type = bitcoin_wallet.network();
    let network = network_type.to_bitcoin_network();
    let testnet = network != bitcoin::Network::Bitcoin;

    // Use nonce=0 for simplicity (could be enhanced with timestamp-based nonce)
    let nonce = 0u64;

    // Delegate to core library
    let generated = core_generate(
        contract_id,
        amount,
        address,
        nonce,
        Consensus::Bitcoin,
        testnet,
    )?;

    Ok(generated)
}

/// Parse RGB invoice string
///
/// Thin passthrough to core library's `parse_invoice()`.
/// No wallet-specific logic needed for parsing.
///
/// # Arguments
///
/// * `invoice_str` - RGB invoice string (format: "rgb:..." or "contract:...")
///
/// # Returns
///
/// `ParsedInvoice` with contract ID, beneficiary, and amount
pub fn parse_invoice(invoice_str: &str) -> Result<ParsedInvoice, InvoiceError> {
    // Direct delegation to core library
    Ok(core_parse(invoice_str)?)
}

/// Extract seal from invoice beneficiary
///
/// Convenience re-export for wallet use.
pub fn extract_seal_from_invoice(
    beneficiary: &RgbBeneficiary,
) -> Result<f1r3fly_rgb::WTxoSeal, InvoiceError> {
    Ok(extract_seal(beneficiary)?)
}

/// Get recipient address from invoice beneficiary
///
/// Convenience re-export for wallet use.
pub fn get_address_from_invoice(
    beneficiary: &RgbBeneficiary,
    network: bitcoin::Network,
) -> Result<String, InvoiceError> {
    Ok(get_recipient_address(beneficiary, network)?)
}

/// Invoice with recipient's public key
///
/// Returns both the RGB invoice string and the recipient's F1r3fly public key.
/// The public key is needed for transfer authorization in RHO20 contracts.
#[derive(Debug, Clone)]
pub struct InvoiceWithPubkey {
    /// Standard RGB invoice string
    pub invoice_string: String,
    /// Recipient's F1r3fly public key (hex-encoded, uncompressed)
    pub recipient_pubkey_hex: String,
}

/// Generate RGB invoice with recipient's public key
///
/// Generates a standard RGB invoice and includes the recipient's F1r3fly public key
/// for transfer authorization. The public key is returned separately (not embedded
/// in the invoice string) due to RGB invoice format limitations.
///
/// # Workflow
///
/// 1. Generate standard RGB invoice (amount only)
/// 2. Get recipient's public key from F1r3fly executor
/// 3. Return both in `InvoiceWithPubkey` struct
///
/// # Arguments
///
/// * `bitcoin_wallet` - Bitcoin wallet to get receiving address from
/// * `contracts_manager` - F1r3fly contracts manager to get public key
/// * `contract_id_str` - RGB contract ID as string
/// * `amount` - Amount to receive (in smallest unit)
///
/// # Returns
///
/// `InvoiceWithPubkey` with invoice string and recipient public key
///
/// # Example
///
/// ```ignore
/// let invoice_data = generate_invoice_with_pubkey(
///     &mut wallet,
///     &contracts_manager,
///     "contract:...",
///     1000,
/// )?;
///
/// // Recipient shares both fields with sender:
/// // - invoice_data.invoice_string (for RGB transfer)
/// // - invoice_data.recipient_pubkey_hex (for RHO20 authorization)
/// ```
pub fn generate_invoice_with_pubkey(
    bitcoin_wallet: &mut BitcoinWallet,
    contracts_manager: &F1r3flyContractsManager,
    contract_id_str: &str,
    amount: u64,
) -> Result<InvoiceWithPubkey, InvoiceError> {
    // Generate standard RGB invoice
    let generated = generate_invoice(bitcoin_wallet, contract_id_str, amount, None)?;

    // Get recipient's F1r3fly public key from executor
    // Each user uses their own F1r3fly key (current derivation index, typically 0)
    let pubkey = contracts_manager
        .contracts()
        .executor()
        .get_public_key()
        .map_err(|e| InvoiceError::Core(e))?;

    let pubkey_hex = hex::encode(pubkey.serialize_uncompressed());

    Ok(InvoiceWithPubkey {
        invoice_string: generated.invoice.to_string(),
        recipient_pubkey_hex: pubkey_hex,
    })
}

// ============================================================================
// Network Utilities
// ============================================================================

impl NetworkType {
    /// Convert NetworkType to bitcoin::Network
    pub fn to_bitcoin_network(&self) -> bitcoin::Network {
        match self {
            NetworkType::Mainnet => bitcoin::Network::Bitcoin,
            NetworkType::Testnet => bitcoin::Network::Testnet,
            NetworkType::Signet => bitcoin::Network::Signet,
            NetworkType::Regtest => bitcoin::Network::Regtest,
        }
    }

    /// Check if this is a testnet/regtest network
    pub fn is_testnet(&self) -> bool {
        matches!(
            self,
            NetworkType::Testnet | NetworkType::Signet | NetworkType::Regtest
        )
    }
}
