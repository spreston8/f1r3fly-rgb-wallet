//! Bitcoin wallet wrapper using BDK

use crate::config::NetworkType;
use bdk_wallet::bitcoin::Network as BdkNetwork;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, PersistedWallet, Wallet};
use std::path::PathBuf;

/// Errors that can occur during Bitcoin wallet operations
#[derive(Debug, thiserror::Error)]
pub enum BitcoinWalletError {
    #[error("BDK descriptor error: {0}")]
    Descriptor(String),

    #[error("BDK create error: {0}")]
    Create(String),

    #[error("BDK load error: {0}")]
    Load(String),

    #[error("Rusqlite error: {0}")]
    Rusqlite(#[from] bdk_wallet::rusqlite::Error),

    #[error("Invalid descriptor: {0}")]
    InvalidDescriptor(String),

    #[error("Network mismatch: expected {expected:?}, got {actual:?}")]
    NetworkMismatch {
        expected: NetworkType,
        actual: NetworkType,
    },

    #[error("Wallet not initialized")]
    NotInitialized,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Bitcoin wallet wrapper around BDK
///
/// Provides Bitcoin UTXO management, transaction creation, and blockchain synchronization
/// using the Bitcoin Development Kit (BDK) library with SQLite persistence.
pub struct BitcoinWallet {
    /// BDK persisted wallet instance with SQLite backend
    wallet: PersistedWallet<Connection>,

    /// SQLite connection for persistence
    conn: Connection,

    /// Network type
    network: NetworkType,

    /// Path to wallet database
    db_path: PathBuf,
}

impl BitcoinWallet {
    /// Create a new Bitcoin wallet from a descriptor with SQLite persistence
    ///
    /// If a wallet already exists at the specified path, it will be loaded.
    /// Otherwise, a new wallet will be created and persisted to SQLite.
    ///
    /// # Arguments
    ///
    /// * `descriptor` - BIP84 descriptor string (e.g., "wpkh(xprv.../0/*)")
    /// * `network` - Network type (Regtest, Signet, Testnet, Mainnet)
    /// * `wallet_dir` - Directory to store wallet database (bitcoin.db)
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::bitcoin::BitcoinWallet;
    /// use f1r3fly_rgb_wallet::config::NetworkType;
    /// use std::path::PathBuf;
    ///
    /// let descriptor = "wpkh(tprv8g8Po8QKfRLF3GM4PA2zFJS9LAknVwxNdgzfHzouQssRbCEvLqTWWjfpfMoRkdXy9V1puyaqnYfaSPxx2ToaC4X1qCefeyCvbu6zGzVroVZ/0/*)".to_string();
    /// let wallet_dir = PathBuf::from("/tmp/test_wallet");
    /// let wallet = BitcoinWallet::new(descriptor, NetworkType::Regtest, &wallet_dir)?;
    /// ```
    pub fn new(
        descriptor: String,
        network: NetworkType,
        wallet_dir: &PathBuf,
    ) -> Result<Self, BitcoinWalletError> {
        // Ensure wallet directory exists
        std::fs::create_dir_all(wallet_dir)?;

        // Convert network type to BDK network
        let bdk_network = network_type_to_bdk(network);

        // Create database path for SQLite persistence
        let db_path = wallet_dir.join("bitcoin.db");

        // Open or create SQLite connection
        let mut conn = Connection::open(&db_path)?;

        // Create internal (change) descriptor by replacing /0/* with /1/*
        // External descriptor format: wpkh(.../ 0/*)
        // Internal descriptor format: wpkh(.../1/*)
        let internal_descriptor = descriptor.replace("/0/*", "/1/*");

        // Try to load existing wallet first, fallback to creating new one
        let wallet = match Wallet::load()
            .descriptor(KeychainKind::External, Some(descriptor.clone()))
            .descriptor(KeychainKind::Internal, Some(internal_descriptor.clone()))
            .extract_keys() // Extract private keys from descriptors for signing
            .load_wallet(&mut conn)
            .map_err(|e| BitcoinWalletError::Load(format!("Failed to load wallet: {}", e)))?
        {
            Some(wallet) => wallet,
            None => {
                // Wallet doesn't exist, create new one
                Wallet::create(descriptor.clone(), internal_descriptor)
                    .network(bdk_network)
                    .create_wallet(&mut conn)
                    .map_err(|e| {
                        BitcoinWalletError::Create(format!("Failed to create wallet: {}", e))
                    })?
            }
        };

        Ok(Self {
            wallet,
            conn,
            network,
            db_path,
        })
    }

    /// Get the underlying BDK wallet reference
    pub fn inner(&self) -> &Wallet {
        &self.wallet
    }

    /// Get mutable reference to the underlying BDK wallet
    pub fn inner_mut(&mut self) -> &mut Wallet {
        &mut self.wallet
    }

    /// Get the network type
    pub fn network(&self) -> NetworkType {
        self.network
    }

    /// Get the database path
    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Persist wallet changes to the SQLite database
    ///
    /// This should be called after operations that modify wallet state
    /// (e.g., syncing with blockchain, generating addresses, creating transactions).
    ///
    /// # Returns
    ///
    /// Returns `true` if changes were persisted, `false` if no changes needed persisting.
    ///
    /// # Example
    ///
    /// ```ignore
    /// wallet.get_new_address()?;
    /// wallet.persist()?; // Save the new address to database
    /// ```
    pub fn persist(&mut self) -> Result<bool, BitcoinWalletError> {
        let changed = self.wallet.persist(&mut self.conn)?;
        Ok(changed)
    }

    /// Get a new receiving address
    ///
    /// # Example
    ///
    /// ```ignore
    /// let address = wallet.get_new_address()?;
    /// println!("New address: {}", address);
    /// ```
    pub fn get_new_address(&mut self) -> Result<bitcoin::Address, BitcoinWalletError> {
        let address_info = self.wallet.reveal_next_address(KeychainKind::External);
        self.persist()?;
        Ok(address_info.address)
    }

    /// Get the next unused address without marking it as used
    ///
    /// # Example
    ///
    /// ```ignore
    /// let address = wallet.peek_address()?;
    /// println!("Next address: {}", address);
    /// ```
    pub fn peek_address(&self) -> Result<bitcoin::Address, BitcoinWalletError> {
        let address_info = self.wallet.peek_address(KeychainKind::External, 0);
        Ok(address_info.address)
    }

    /// List all addresses (up to a certain index)
    ///
    /// # Arguments
    ///
    /// * `count` - Number of addresses to return
    ///
    /// # Example
    ///
    /// ```ignore
    /// let addresses = wallet.list_addresses(5)?;
    /// for (index, address) in addresses.iter().enumerate() {
    ///     println!("{}: {}", index, address);
    /// }
    /// ```
    pub fn list_addresses(&self, count: u32) -> Result<Vec<bitcoin::Address>, BitcoinWalletError> {
        let mut addresses = Vec::new();
        for index in 0..count {
            let address_info = self.wallet.peek_address(KeychainKind::External, index);
            addresses.push(address_info.address);
        }
        Ok(addresses)
    }

    /// List all UTXOs with basic information (Bitcoin-only, no RGB data)
    ///
    /// Returns all unspent outputs controlled by this wallet, including confirmation
    /// counts calculated from the current blockchain height. RGB occupation status
    /// is set to Available by default (RGB enrichment happens in manager layer).
    ///
    /// # Returns
    ///
    /// Vector of `crate::types::UtxoInfo` with Bitcoin data only.
    ///
    /// # Errors
    ///
    /// Returns error if wallet queries fail.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::bitcoin::BitcoinWallet;
    ///
    /// let utxos = wallet.list_all_utxos()?;
    /// for utxo in utxos {
    ///     println!("{}: {} BTC (confirmations: {})",
    ///         utxo.outpoint,
    ///         utxo.amount_btc,
    ///         utxo.confirmations
    ///     );
    /// }
    /// ```
    pub fn list_all_utxos(&self) -> Result<Vec<crate::types::UtxoInfo>, BitcoinWalletError> {
        use crate::bitcoin::balance;
        use crate::types::{UtxoInfo, UtxoStatus};
        use std::collections::HashSet;

        // Get blockchain height for confirmation calculation
        let local_chain = self.wallet.local_chain();
        let current_height = local_chain.tip().height();

        // Use existing list_utxos function with empty RGB set (no RGB data yet)
        let rgb_occupied = HashSet::new();
        let bdk_utxos = balance::list_utxos(self, &rgb_occupied)
            .map_err(|e| BitcoinWalletError::Load(format!("Failed to list UTXOs: {}", e)))?;

        // Convert BDK UTXOs to our types::UtxoInfo
        let mut utxos: Vec<UtxoInfo> = bdk_utxos
            .into_iter()
            .map(|utxo| {
                // Calculate confirmations from blockchain height
                let confirmations = if let Some(conf_height) = utxo.confirmation_height {
                    // Confirmed: current_height - conf_height + 1
                    current_height.saturating_sub(conf_height).saturating_add(1)
                } else {
                    // Unconfirmed: 0 confirmations
                    0
                };

                // Determine status based on confirmation
                let status = if confirmations == 0 {
                    UtxoStatus::Unconfirmed
                } else {
                    UtxoStatus::Available
                };

                // Format outpoint as "txid:vout"
                let outpoint = format!("{}:{}", utxo.outpoint.txid, utxo.outpoint.vout);
                let txid = utxo.outpoint.txid.to_string();

                // Convert satoshis to BTC for display
                let amount_btc = utxo.amount as f64 / 100_000_000.0;

                UtxoInfo {
                    outpoint,
                    txid,
                    vout: utxo.outpoint.vout,
                    amount_sats: utxo.amount,
                    amount_btc,
                    confirmations,
                    status,
                    rgb_assets: vec![], // No RGB data at Bitcoin layer
                }
            })
            .collect();

        // Sort by confirmations descending (most confirmed first)
        utxos.sort_by(|a, b| b.confirmations.cmp(&a.confirmations));

        Ok(utxos)
    }
}

/// Convert NetworkType to BDK Network
fn network_type_to_bdk(network: NetworkType) -> BdkNetwork {
    match network {
        NetworkType::Mainnet => BdkNetwork::Bitcoin,
        NetworkType::Testnet => BdkNetwork::Testnet,
        NetworkType::Signet => BdkNetwork::Signet,
        NetworkType::Regtest => BdkNetwork::Regtest,
    }
}
