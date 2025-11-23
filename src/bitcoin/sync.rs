//! Blockchain synchronization operations

use crate::bitcoin::{BitcoinWallet, BitcoinWalletError, EsploraClient, NetworkError};
use bdk_esplora::EsploraExt;
use bdk_wallet::bitcoin::BlockHash;
use bdk_wallet::KeychainKind;

/// Errors that can occur during sync operations
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Wallet error: {0}")]
    Wallet(#[from] BitcoinWalletError),

    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    #[error("Esplora sync error: {0}")]
    Esplora(String),

    #[error("Sync failed: {0}")]
    Failed(String),
}

/// Result of a blockchain sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Current blockchain height after sync
    pub height: u32,

    /// Current tip block hash
    pub tip_hash: BlockHash,

    /// Number of new transactions discovered
    pub new_txs: usize,

    /// Number of transactions updated
    pub updated_txs: usize,

    /// Number of new addresses revealed
    pub new_addresses: u32,
}

impl SyncResult {
    /// Check if any new data was discovered during sync
    pub fn has_updates(&self) -> bool {
        self.new_txs > 0 || self.updated_txs > 0 || self.new_addresses > 0
    }
}

/// Sync wallet with the blockchain via Esplora
///
/// Performs a full sync with the blockchain, discovering new transactions,
/// updating transaction confirmations, and revealing new addresses as needed.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet to sync
/// * `client` - The Esplora client for blockchain queries
///
/// # Returns
///
/// Returns `SyncResult` with details about what was synced.
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::bitcoin::{BitcoinWallet, EsploraClient, sync_wallet};
///
/// let mut wallet = BitcoinWallet::new(descriptor, network, &wallet_dir)?;
/// let client = EsploraClient::new_with_default_url(network)?;
///
/// let result = sync_wallet(&mut wallet, &client)?;
/// println!("Synced to height: {}", result.height);
/// println!("New transactions: {}", result.new_txs);
/// ```
pub fn sync_wallet(
    wallet: &mut BitcoinWallet,
    client: &EsploraClient,
) -> Result<SyncResult, SyncError> {
    log::info!("Starting blockchain sync...");

    // Get current height before sync
    let height_before = wallet.inner().latest_checkpoint().height();
    log::debug!("Current wallet height: {}", height_before);

    // Get blockchain tip
    let tip_height = client.get_height()?;
    let tip_hash = client.get_tip_hash()?;
    log::info!("Blockchain tip: {} (height: {})", tip_hash, tip_height);

    // Build sync request with revealed script pubkeys
    let request = wallet.inner().start_sync_with_revealed_spks();

    // DIAGNOSTIC: Log what addresses BDK is actually tracking
    let derivation_index = wallet.inner().derivation_index(KeychainKind::External);
    if let Some(last_index) = derivation_index {
        log::debug!(
            "🔍 BDK External Derivation Index: {} (will scan 0..={})",
            last_index,
            last_index
        );

        // Sample first few addresses to see what BDK knows about
        for index in 0..=last_index.min(5) {
            let addr_info = wallet.inner().peek_address(KeychainKind::External, index);
            log::debug!("  - Index {}: {} (revealed)", index, addr_info.address);
        }
    } else {
        log::debug!("🔍 BDK External Derivation Index: None (no addresses revealed yet)");
    }

    log::debug!("Syncing with revealed script pubkeys");

    // Perform full scan
    log::info!("Querying blockchain data...");

    // Get transaction count before sync
    let txs_before = wallet.inner().transactions().count();

    let update = client
        .inner()
        .sync(request, 5) // parallel_requests = 5
        .map_err(|e| SyncError::Esplora(format!("Sync failed: {}", e)))?;

    // Apply update to wallet
    log::debug!("Applying sync update to wallet...");
    wallet
        .inner_mut()
        .apply_update(update)
        .map_err(|e| SyncError::Failed(format!("Failed to apply update: {}", e)))?;

    // Count transactions after sync
    let txs_after = wallet.inner().transactions().count();
    let new_txs = txs_after.saturating_sub(txs_before);
    let updated_txs = 0; // BDK handles this internally

    if new_txs > 0 {
        log::info!("Discovered {} new transactions", new_txs);
    }

    // Persist wallet changes
    log::debug!("Persisting wallet changes...");
    let persisted = wallet.persist()?;
    if persisted {
        log::debug!("Wallet state persisted to database");
    }

    // Get final height and calculate new addresses
    let height_after = wallet.inner().latest_checkpoint().height();
    let new_addresses = height_after.saturating_sub(height_before);

    // Build result
    let result = SyncResult {
        height: tip_height,
        tip_hash,
        new_txs,
        updated_txs,
        new_addresses,
    };

    log::info!(
        "Sync complete: height={}, new_txs={}, updated_txs={}, new_addresses={}",
        result.height,
        result.new_txs,
        result.updated_txs,
        result.new_addresses
    );

    Ok(result)
}

/// Sync wallet with progress callback
///
/// Similar to `sync_wallet` but allows for progress reporting via callback.
///
/// # Arguments
///
/// * `wallet` - The Bitcoin wallet to sync
/// * `client` - The Esplora client for blockchain queries
/// * `progress_fn` - Callback function called with progress updates
///
/// # Example
///
/// ```ignore
/// sync_wallet_with_progress(&mut wallet, &client, |msg| {
///     println!("Progress: {}", msg);
/// })?;
/// ```
pub fn sync_wallet_with_progress<F>(
    wallet: &mut BitcoinWallet,
    client: &EsploraClient,
    mut progress_fn: F,
) -> Result<SyncResult, SyncError>
where
    F: FnMut(&str),
{
    progress_fn("Starting blockchain sync...");

    let height_before = wallet.inner().latest_checkpoint().height();
    let tip_height = client.get_height()?;
    let tip_hash = client.get_tip_hash()?;

    progress_fn(&format!("Syncing to height: {}", tip_height));

    let request = wallet.inner().start_sync_with_revealed_spks();
    progress_fn("Querying script pubkeys...");

    // Get transaction count before sync
    let txs_before = wallet.inner().transactions().count();

    let update = client
        .inner()
        .sync(request, 5)
        .map_err(|e| SyncError::Esplora(format!("Sync failed: {}", e)))?;

    progress_fn("Applying updates to wallet...");
    wallet
        .inner_mut()
        .apply_update(update)
        .map_err(|e| SyncError::Failed(format!("Failed to apply update: {}", e)))?;

    // Count transactions after sync
    let txs_after = wallet.inner().transactions().count();
    let new_txs = txs_after.saturating_sub(txs_before);
    let updated_txs = 0; // BDK handles this internally

    if new_txs > 0 {
        progress_fn(&format!("Found {} new transactions", new_txs));
    }

    progress_fn("Persisting wallet state...");
    wallet.persist()?;

    let height_after = wallet.inner().latest_checkpoint().height();
    let new_addresses = height_after.saturating_sub(height_before);

    let result = SyncResult {
        height: tip_height,
        tip_hash,
        new_txs,
        updated_txs,
        new_addresses,
    };

    progress_fn(&format!("Sync complete! Height: {}", result.height));

    Ok(result)
}
