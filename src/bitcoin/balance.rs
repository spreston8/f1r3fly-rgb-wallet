//! Bitcoin wallet balance and UTXO queries

use crate::bitcoin::{BitcoinWallet, BitcoinWalletError};
use bdk_wallet::bitcoin::{OutPoint, TxOut};
use bdk_wallet::KeychainKind;
use std::collections::HashSet;

/// Errors that can occur during balance operations
#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error("Wallet error: {0}")]
    Wallet(#[from] BitcoinWalletError),

    #[error("UTXO not found: {0}")]
    UtxoNotFound(String),

    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
}

/// Bitcoin balance information
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Balance {
    /// Total confirmed balance (in satoshis)
    pub confirmed: u64,

    /// Total unconfirmed balance (in satoshis)
    pub unconfirmed: u64,

    /// Total balance (confirmed + unconfirmed)
    pub total: u64,
}

impl Balance {
    /// Create a new balance
    pub fn new(confirmed: u64, unconfirmed: u64) -> Self {
        Self {
            confirmed,
            unconfirmed,
            total: confirmed + unconfirmed,
        }
    }

    /// Check if the wallet has any balance
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Get the spendable balance (confirmed only)
    pub fn spendable(&self) -> u64 {
        self.confirmed
    }
}

/// UTXO information
#[derive(Debug, Clone)]
pub struct UtxoInfo {
    /// Output point (txid:vout)
    pub outpoint: OutPoint,

    /// Transaction output
    pub txout: TxOut,

    /// Amount in satoshis
    pub amount: u64,

    /// Whether this UTXO is confirmed
    pub is_confirmed: bool,

    /// Block height where this UTXO was confirmed (if confirmed)
    pub confirmation_height: Option<u32>,

    /// Keychain kind (External/Internal)
    pub keychain: KeychainKind,

    /// Derivation index
    pub derivation_index: u32,

    /// Whether this UTXO is marked as RGB-occupied
    pub is_rgb_occupied: bool,
}

impl UtxoInfo {
    /// Check if this UTXO is spendable
    pub fn is_spendable(&self) -> bool {
        self.is_confirmed && !self.is_rgb_occupied
    }

    /// Get the UTXO identifier as string
    pub fn identifier(&self) -> String {
        format!("{}:{}", self.outpoint.txid, self.outpoint.vout)
    }
}

/// Address information
#[derive(Debug, Clone)]
pub struct AddressInfo {
    /// Bitcoin address
    pub address: bdk_wallet::bitcoin::Address,

    /// Keychain kind (External/Internal)
    pub keychain: KeychainKind,

    /// Derivation index
    pub index: u32,

    /// Whether this address has been used
    pub is_used: bool,
}

/// Get wallet balance
///
/// Returns confirmed and unconfirmed Bitcoin balance.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::bitcoin::{BitcoinWallet, get_balance};
///
/// let balance = get_balance(&wallet)?;
/// println!("Confirmed: {} sats", balance.confirmed);
/// println!("Unconfirmed: {} sats", balance.unconfirmed);
/// println!("Total: {} sats", balance.total);
/// ```
pub fn get_balance(wallet: &BitcoinWallet) -> Result<Balance, BalanceError> {
    let bdk_balance = wallet.inner().balance();

    let confirmed = bdk_balance.confirmed.to_sat();
    let unconfirmed = bdk_balance.trusted_pending.to_sat() + bdk_balance.untrusted_pending.to_sat();

    Ok(Balance::new(confirmed, unconfirmed))
}

/// List all wallet UTXOs with details
///
/// Returns a list of all UTXOs controlled by the wallet, including
/// confirmation status and derivation information.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `rgb_occupied` - Set of outpoints marked as RGB-occupied
///
/// # Example
///
/// ```ignore
/// use std::collections::HashSet;
///
/// let rgb_occupied = HashSet::new();
/// let utxos = list_utxos(&wallet, &rgb_occupied)?;
///
/// for utxo in &utxos {
///     println!("{}: {} sats (confirmed: {})",
///         utxo.identifier(),
///         utxo.amount,
///         utxo.is_confirmed
///     );
/// }
/// ```
pub fn list_utxos(
    wallet: &BitcoinWallet,
    rgb_occupied: &HashSet<OutPoint>,
) -> Result<Vec<UtxoInfo>, BalanceError> {
    let mut utxos = Vec::new();

    // Iterate through all unspent outputs
    for utxo in wallet.inner().list_unspent() {
        let is_confirmed = utxo.chain_position.is_confirmed();
        let confirmation_height = match utxo.chain_position {
            bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. } => {
                Some(anchor.block_id.height)
            }
            _ => None,
        };

        let utxo_info = UtxoInfo {
            outpoint: utxo.outpoint,
            txout: utxo.txout.clone(),
            amount: utxo.txout.value.to_sat(),
            is_confirmed,
            confirmation_height,
            keychain: utxo.keychain,
            derivation_index: utxo.derivation_index,
            is_rgb_occupied: rgb_occupied.contains(&utxo.outpoint),
        };

        utxos.push(utxo_info);
    }

    // Sort by amount descending
    utxos.sort_by(|a, b| b.amount.cmp(&a.amount));

    Ok(utxos)
}

/// Mark UTXOs as RGB-occupied
///
/// Updates the set of outpoints that are holding RGB assets.
/// These UTXOs should not be spent for regular Bitcoin transactions.
///
/// # Arguments
///
/// * `rgb_occupied` - Mutable set of RGB-occupied outpoints
/// * `outpoints` - Iterator of outpoints to mark as occupied
///
/// # Example
///
/// ```ignore
/// use std::collections::HashSet;
/// use bdk_wallet::bitcoin::OutPoint;
///
/// let mut rgb_occupied = HashSet::new();
/// let outpoints = vec![outpoint1, outpoint2];
///
/// mark_rgb_occupied(&mut rgb_occupied, outpoints.into_iter());
/// assert_eq!(rgb_occupied.len(), 2);
/// ```
pub fn mark_rgb_occupied<I>(rgb_occupied: &mut HashSet<OutPoint>, outpoints: I)
where
    I: IntoIterator<Item = OutPoint>,
{
    for outpoint in outpoints {
        rgb_occupied.insert(outpoint);
    }
}

/// Unmark UTXOs as RGB-occupied
///
/// Removes outpoints from the RGB-occupied set, making them available
/// for regular Bitcoin transactions again.
///
/// # Arguments
///
/// * `rgb_occupied` - Mutable set of RGB-occupied outpoints
/// * `outpoints` - Iterator of outpoints to unmark
///
/// # Example
///
/// ```ignore
/// unmark_rgb_occupied(&mut rgb_occupied, vec![outpoint1].into_iter());
/// ```
pub fn unmark_rgb_occupied<I>(rgb_occupied: &mut HashSet<OutPoint>, outpoints: I)
where
    I: IntoIterator<Item = OutPoint>,
{
    for outpoint in outpoints {
        rgb_occupied.remove(&outpoint);
    }
}

/// Check if a UTXO is RGB-occupied
///
/// # Arguments
///
/// * `rgb_occupied` - Set of RGB-occupied outpoints
/// * `outpoint` - Outpoint to check
///
/// # Example
///
/// ```ignore
/// if is_rgb_occupied(&rgb_occupied, &outpoint) {
///     println!("UTXO is holding RGB assets");
/// }
/// ```
pub fn is_rgb_occupied(rgb_occupied: &HashSet<OutPoint>, outpoint: &OutPoint) -> bool {
    rgb_occupied.contains(outpoint)
}

/// Get wallet addresses
///
/// Returns a list of addresses derived by the wallet, including
/// both used and unused addresses.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `count` - Number of addresses to return per keychain (None = all revealed)
///
/// # Example
///
/// ```ignore
/// // Get all revealed addresses
/// let addresses = get_addresses(&wallet, None)?;
///
/// // Get first 10 addresses
/// let addresses = get_addresses(&wallet, Some(10))?;
///
/// for addr_info in &addresses {
///     println!("{} (index: {}, used: {})",
///         addr_info.address,
///         addr_info.index,
///         addr_info.is_used
///     );
/// }
/// ```
pub fn get_addresses(
    wallet: &mut BitcoinWallet,
    count: Option<u32>,
) -> Result<Vec<AddressInfo>, BalanceError> {
    let mut addresses = Vec::new();

    // Get external (receive) addresses
    // IMPORTANT: Only return already-revealed addresses, don't reveal new ones
    // Revealing should be done explicitly via get_new_address()
    let external_count = count.unwrap_or(100); // Default to 100 if not specified

    for index in 0..external_count {
        let addr_info = wallet.inner().peek_address(KeychainKind::External, index);
        let is_used = wallet
            .inner()
            .derivation_index(KeychainKind::External)
            .map(|last_index| index <= last_index)
            .unwrap_or(false);

        addresses.push(AddressInfo {
            address: addr_info.address,
            keychain: KeychainKind::External,
            index,
            is_used,
        });

        // If count is specified, only get that many
        if count.is_some() {
            continue;
        }

        // If no count specified, stop at first unused address
        if !is_used {
            break;
        }
    }

    // Get internal (change) addresses
    let internal_count = count.unwrap_or(100);

    for index in 0..internal_count {
        let addr_info = wallet.inner().peek_address(KeychainKind::Internal, index);
        let is_used = wallet
            .inner()
            .derivation_index(KeychainKind::Internal)
            .map(|last_index| index <= last_index)
            .unwrap_or(false);

        addresses.push(AddressInfo {
            address: addr_info.address,
            keychain: KeychainKind::Internal,
            index,
            is_used,
        });

        // If count is specified, only get that many
        if count.is_some() {
            continue;
        }

        // If no count specified, stop at first unused address
        if !is_used {
            break;
        }
    }

    Ok(addresses)
}

/// Get only unused addresses
///
/// Returns addresses that have not been used yet.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet
/// * `keychain` - Optional keychain filter (External/Internal)
///
/// # Example
///
/// ```ignore
/// // Get all unused addresses
/// let unused = get_unused_addresses(&wallet, None)?;
///
/// // Get only unused external (receive) addresses
/// let unused_external = get_unused_addresses(&wallet, Some(KeychainKind::External))?;
/// ```
pub fn get_unused_addresses(
    wallet: &mut BitcoinWallet,
    keychain: Option<KeychainKind>,
) -> Result<Vec<AddressInfo>, BalanceError> {
    let addresses = get_addresses(wallet, None)?;

    let unused: Vec<AddressInfo> = addresses
        .into_iter()
        .filter(|addr| !addr.is_used)
        .filter(|addr| keychain.map_or(true, |k| addr.keychain == k))
        .collect();

    Ok(unused)
}
