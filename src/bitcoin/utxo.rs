//! Bitcoin UTXO management operations
//!
//! Handles creating and unlocking specific UTXOs for RGB asset management

use crate::bitcoin::{BitcoinWallet, BitcoinWalletError, EsploraClient, NetworkError};
use bdk_wallet::bitcoin::{Amount, FeeRate, OutPoint};
#[allow(deprecated)]
use bdk_wallet::{KeychainKind, SignOptions};
use std::collections::HashSet;

/// Errors that can occur during UTXO operations
#[derive(Debug, thiserror::Error)]
pub enum UtxoError {
    #[error("Wallet error: {0}")]
    Wallet(#[from] BitcoinWalletError),

    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    #[error("Insufficient funds: need {needed} sats, have {available} sats")]
    InsufficientFunds { needed: u64, available: u64 },

    #[error("Invalid amount: {0}")]
    InvalidAmount(String),

    #[error("Invalid fee rate: {0}")]
    InvalidFeeRate(String),

    #[error("Transaction build failed: {0}")]
    BuildFailed(String),

    #[error("Transaction sign failed: {0}")]
    SignFailed(String),

    #[error("Transaction broadcast failed: {0}")]
    BroadcastFailed(String),

    #[error("UTXO not found: {0}")]
    UtxoNotFound(String),
}

/// Result of a UTXO operation
#[derive(Debug, Clone)]
pub struct UtxoOperationResult {
    /// Transaction ID
    pub txid: String,

    /// Created or unlocked outpoint
    pub outpoint: OutPoint,

    /// Amount in satoshis
    pub amount: u64,

    /// Fee paid in satoshis
    pub fee: u64,

    /// Fee rate in sat/vB
    pub fee_rate: f64,
}

impl UtxoOperationResult {
    /// Create a new result
    pub fn new(txid: String, vout: u32, amount: u64, fee: u64, fee_rate: f64) -> Self {
        let outpoint = OutPoint {
            txid: txid.parse().expect("Invalid txid"),
            vout,
        };

        Self {
            txid,
            outpoint,
            amount,
            fee,
            fee_rate,
        }
    }

    /// Get the outpoint identifier
    pub fn outpoint_id(&self) -> String {
        format!("{}:{}", self.txid, self.outpoint.vout)
    }
}

/// Fee rate configuration
#[derive(Debug, Clone, Copy)]
pub struct FeeRateConfig {
    /// Fee rate in satoshis per virtual byte (sat/vB)
    pub sat_per_vb: f64,
}

impl FeeRateConfig {
    /// Create a new fee rate configuration
    pub fn new(sat_per_vb: f64) -> Result<Self, UtxoError> {
        if sat_per_vb <= 0.0 {
            return Err(UtxoError::InvalidFeeRate(
                "Fee rate must be positive".to_string(),
            ));
        }

        Ok(Self { sat_per_vb })
    }

    /// Create a fee rate for low priority (1 sat/vB)
    pub fn low_priority() -> Self {
        Self { sat_per_vb: 1.0 }
    }

    /// Create a fee rate for medium priority (5 sat/vB)
    pub fn medium_priority() -> Self {
        Self { sat_per_vb: 5.0 }
    }

    /// Create a fee rate for high priority (10 sat/vB)
    pub fn high_priority() -> Self {
        Self { sat_per_vb: 10.0 }
    }

    /// Convert to BDK FeeRate
    pub fn to_bdk_fee_rate(&self) -> FeeRate {
        // BDK FeeRate expects sat/kwu (satoshis per 1000 weight units)
        // 1 vByte = 4 weight units, so 1 sat/vB = 4000 sat/kwu
        let sat_per_kwu = (self.sat_per_vb * 4000.0) as u64;
        FeeRate::from_sat_per_kwu(sat_per_kwu)
    }
}

/// Calculate estimated fee for a transaction
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `amount` - Amount to send in satoshis
/// * `fee_rate` - Fee rate configuration
///
/// # Returns
///
/// Estimated fee in satoshis
pub fn estimate_fee(
    wallet: &mut BitcoinWallet,
    amount: u64,
    fee_rate: &FeeRateConfig,
) -> Result<u64, UtxoError> {
    // Get a change address
    let change_address = wallet
        .inner_mut()
        .next_unused_address(KeychainKind::Internal)
        .address;

    // Build a transaction to estimate fee
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(change_address.script_pubkey(), Amount::from_sat(amount));
    tx_builder.fee_rate(fee_rate.to_bdk_fee_rate());

    match tx_builder.finish() {
        Ok(psbt) => {
            // Calculate fee from the PSBT
            let fee = psbt
                .fee()
                .map_err(|e| UtxoError::BuildFailed(format!("Failed to calculate fee: {}", e)))?;
            Ok(fee.to_sat())
        }
        Err(e) => Err(UtxoError::BuildFailed(format!(
            "Failed to estimate fee: {}",
            e
        ))),
    }
}

/// Create a new UTXO by self-sending Bitcoin
///
/// This creates a transaction that sends a specific amount to a new address
/// controlled by the wallet. This is useful for creating UTXOs that will hold
/// RGB assets.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `client` - Esplora client for broadcasting
/// * `amount` - Amount in satoshis for the new UTXO
/// * `fee_rate` - Fee rate configuration
/// * `rgb_occupied` - Optional set to mark the new UTXO as RGB-occupied
///
/// # Returns
///
/// Result containing transaction details and the new outpoint
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::bitcoin::{create_utxo, FeeRateConfig};
///
/// let amount = 10_000; // 10,000 sats
/// let fee_rate = FeeRateConfig::medium_priority();
/// let result = create_utxo(&mut wallet, &client, amount, &fee_rate, None)?;
///
/// println!("Created UTXO: {}", result.outpoint_id());
/// println!("Fee paid: {} sats", result.fee);
/// ```
pub fn create_utxo(
    wallet: &mut BitcoinWallet,
    client: &EsploraClient,
    amount: u64,
    fee_rate: &FeeRateConfig,
    rgb_occupied: Option<&mut HashSet<OutPoint>>,
    mark_output_as_rgb: bool,
) -> Result<UtxoOperationResult, UtxoError> {
    // Validate amount
    if amount == 0 {
        return Err(UtxoError::InvalidAmount(
            "Amount must be greater than 0".to_string(),
        ));
    }

    // Get a new address to receive the UTXO
    let address_info = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let address = address_info.address;

    // Build the transaction
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(address.script_pubkey(), Amount::from_sat(amount));
    tx_builder.fee_rate(fee_rate.to_bdk_fee_rate());

    // Exclude RGB-occupied UTXOs from being spent as inputs
    if let Some(rgb_set) = rgb_occupied.as_ref() {
        for occupied_outpoint in rgb_set.iter() {
            tx_builder.add_unspendable(*occupied_outpoint);
        }
    }

    // Finish building the PSBT
    let mut psbt = tx_builder
        .finish()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to build transaction: {}", e)))?;

    // Sign the PSBT
    #[allow(deprecated)]
    wallet
        .inner_mut()
        .sign(&mut psbt, SignOptions::default())
        .map_err(|e| UtxoError::SignFailed(format!("Failed to sign transaction: {}", e)))?;

    // Calculate fee before extracting (extract_tx consumes the PSBT)
    let fee = psbt
        .fee()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to calculate fee: {}", e)))?
        .to_sat();

    // Extract the final transaction
    let tx = psbt
        .extract_tx()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to extract transaction: {}", e)))?;

    // Broadcast the transaction
    client
        .inner()
        .broadcast(&tx)
        .map_err(|e| UtxoError::BroadcastFailed(format!("Failed to broadcast: {}", e)))?;

    // Find the output index for our address
    let vout = tx
        .output
        .iter()
        .position(|output| output.script_pubkey == address.script_pubkey())
        .ok_or_else(|| UtxoError::BuildFailed("Could not find output in transaction".to_string()))?
        as u32;

    let txid = tx.compute_txid().to_string();
    let outpoint = OutPoint {
        txid: tx.compute_txid(),
        vout,
    };

    // Mark output as RGB-occupied if requested
    if mark_output_as_rgb {
        if let Some(rgb_set) = rgb_occupied {
            rgb_set.insert(outpoint);
        }
    }

    // Persist wallet changes
    wallet.persist()?;

    // Calculate effective fee rate
    let tx_vsize = tx.vsize() as f64;
    let effective_fee_rate = fee as f64 / tx_vsize;

    Ok(UtxoOperationResult::new(
        txid,
        vout,
        amount,
        fee,
        effective_fee_rate,
    ))
}

/// Unlock a UTXO by spending it back to self
///
/// This spends a specific UTXO and sends the funds to a new address controlled
/// by the wallet. This is useful for "unlocking" UTXOs that were holding RGB
/// assets but are no longer needed for that purpose.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `client` - Esplora client for broadcasting
/// * `outpoint` - The outpoint to unlock
/// * `fee_rate` - Fee rate configuration
/// * `rgb_occupied` - Optional set to unmark the UTXO as RGB-occupied
///
/// # Returns
///
/// Result containing transaction details and the new outpoint
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::bitcoin::{unlock_utxo, FeeRateConfig};
///
/// let outpoint = OutPoint { txid, vout: 0 };
/// let fee_rate = FeeRateConfig::medium_priority();
/// let result = unlock_utxo(&mut wallet, &client, outpoint, &fee_rate, None)?;
///
/// println!("Unlocked UTXO: {}", result.outpoint_id());
/// println!("Fee paid: {} sats", result.fee);
/// ```
pub fn unlock_utxo(
    wallet: &mut BitcoinWallet,
    client: &EsploraClient,
    outpoint: OutPoint,
    fee_rate: &FeeRateConfig,
    rgb_occupied: Option<&mut HashSet<OutPoint>>,
) -> Result<UtxoOperationResult, UtxoError> {
    // Find the UTXO in the wallet
    let utxo = wallet
        .inner()
        .list_unspent()
        .find(|u| u.outpoint == outpoint)
        .ok_or_else(|| UtxoError::UtxoNotFound(format!("{}:{}", outpoint.txid, outpoint.vout)))?;

    let utxo_amount = utxo.txout.value.to_sat();

    // Get a new address to receive the unlocked funds
    let address_info = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let address = address_info.address;

    // Build the transaction, manually adding the specific UTXO
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder
        .add_utxo(outpoint)
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to add UTXO: {}", e)))?;
    tx_builder.manually_selected_only();
    tx_builder.drain_to(address.script_pubkey());
    tx_builder.fee_rate(fee_rate.to_bdk_fee_rate());

    // Finish building the PSBT
    let mut psbt = tx_builder
        .finish()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to build transaction: {}", e)))?;

    // Sign the PSBT
    #[allow(deprecated)]
    wallet
        .inner_mut()
        .sign(&mut psbt, SignOptions::default())
        .map_err(|e| UtxoError::SignFailed(format!("Failed to sign transaction: {}", e)))?;

    // Calculate fee before extracting (extract_tx consumes the PSBT)
    let fee = psbt
        .fee()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to calculate fee: {}", e)))?
        .to_sat();

    // Extract the final transaction
    let tx = psbt
        .extract_tx()
        .map_err(|e| UtxoError::BuildFailed(format!("Failed to extract transaction: {}", e)))?;

    // Broadcast the transaction
    client
        .inner()
        .broadcast(&tx)
        .map_err(|e| UtxoError::BroadcastFailed(format!("Failed to broadcast: {}", e)))?;

    // Find the output index for our address (should be output 0 for drain_to)
    let vout = tx
        .output
        .iter()
        .position(|output| output.script_pubkey == address.script_pubkey())
        .ok_or_else(|| UtxoError::BuildFailed("Could not find output in transaction".to_string()))?
        as u32;

    let txid = tx.compute_txid().to_string();

    // Calculate the new UTXO amount (original - fee)
    let new_amount = utxo_amount.saturating_sub(fee);

    // Unmark the old UTXO if requested
    if let Some(rgb_set) = rgb_occupied {
        rgb_set.remove(&outpoint);
        // Note: We don't automatically mark the new UTXO as occupied
        // since the purpose of unlocking is typically to free it up
    }

    // Persist wallet changes
    wallet.persist()?;

    // Calculate effective fee rate
    let tx_vsize = tx.vsize() as f64;
    let effective_fee_rate = fee as f64 / tx_vsize;

    Ok(UtxoOperationResult::new(
        txid,
        vout,
        new_amount,
        fee,
        effective_fee_rate,
    ))
}

/// Get recommended fee rates from network
///
/// Queries the Esplora API for current fee rate estimates.
///
/// # Arguments
///
/// * `client` - Esplora client
///
/// # Returns
///
/// Tuple of (low_priority, medium_priority, high_priority) fee rates
///
/// # Example
///
/// ```ignore
/// let (low, medium, high) = get_recommended_fee_rates(&client)?;
/// println!("Fee rates - Low: {}, Medium: {}, High: {}", low.sat_per_vb, medium.sat_per_vb, high.sat_per_vb);
/// ```
pub fn get_recommended_fee_rates(
    client: &EsploraClient,
) -> Result<(FeeRateConfig, FeeRateConfig, FeeRateConfig), UtxoError> {
    // Get fee estimates from Esplora
    let fee_estimates = client
        .inner()
        .get_fee_estimates()
        .map_err(|e| NetworkError::Request(format!("Failed to get fee estimates: {}", e)))?;

    // Esplora returns estimates as a map of confirmation targets to fee rates (sat/vB)
    // Common targets: 1 block (high), 3 blocks (medium), 6 blocks (low)
    let high_priority = fee_estimates.get(&1).copied().unwrap_or(10.0);

    let medium_priority = fee_estimates.get(&3).copied().unwrap_or(5.0);

    let low_priority = fee_estimates.get(&6).copied().unwrap_or(1.0);

    Ok((
        FeeRateConfig::new(low_priority)?,
        FeeRateConfig::new(medium_priority)?,
        FeeRateConfig::new(high_priority)?,
    ))
}
