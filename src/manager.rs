//! Wallet manager - Main integration layer
//!
//! Coordinates between config, storage, Bitcoin, and F1r3fly layers

use crate::bitcoin::{
    create_utxo, get_addresses, get_balance, sync_wallet, AddressInfo, Balance, BalanceError,
    BitcoinWallet, BitcoinWalletError, EsploraClient, FeeRateConfig, NetworkError, SyncError,
    SyncResult, UtxoError, UtxoOperationResult,
};
use crate::config::{ConfigError, GlobalConfig};
use crate::f1r3fly::balance::BalanceError as RgbBalanceError;
use crate::f1r3fly::executor::F1r3flyExecutorError;
use crate::f1r3fly::{
    get_asset_balance, get_asset_info, get_occupied_utxos, get_rgb_balance, get_rgb_seal_info,
    issue_asset, list_assets, AssetBalance, AssetError, AssetInfo, AssetListItem,
    ContractsManagerError, F1r3flyContractsManager, F1r3flyExecutorManager, IssueAssetRequest,
    RgbOccupiedUtxo,
};
use crate::storage::{
    file_system::{create_wallet_directory, load_wallet, save_wallet, wallet_dir, FileSystemError},
    keys::{generate_mnemonic, KeyError},
    models::{WalletKeys, WalletMetadata},
};
use crate::types::{UtxoFilter, UtxoInfo, UtxoStatus};
use bdk_wallet::bitcoin::OutPoint;
#[allow(deprecated)]
use bdk_wallet::{KeychainKind, SignOptions};
use std::collections::HashSet;

/// Errors that can occur in the wallet manager
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("File system error: {0}")]
    FileSystem(#[from] FileSystemError),

    #[error("Key error: {0}")]
    Key(#[from] KeyError),

    #[error("Bitcoin wallet error: {0}")]
    BitcoinWallet(#[from] BitcoinWalletError),

    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    #[error("Sync error: {0}")]
    Sync(#[from] SyncError),

    #[error("Balance error: {0}")]
    Balance(#[from] BalanceError),

    #[error("UTXO error: {0}")]
    Utxo(#[from] UtxoError),

    #[error("Asset error: {0}")]
    Asset(#[from] AssetError),

    #[error("Contracts manager error: {0}")]
    ContractsManager(#[from] ContractsManagerError),

    #[error("F1r3fly executor error: {0}")]
    F1r3flyExecutor(#[from] F1r3flyExecutorError),

    #[error("RGB balance error: {0}")]
    RgbBalance(#[from] RgbBalanceError),

    #[error("Wallet not loaded")]
    WalletNotLoaded,

    #[error("Wallet already exists: {0}")]
    WalletAlreadyExists(String),

    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("F1r3fly not initialized - wallet must be loaded first")]
    F1r3flyNotInitialized,
}

/// Main wallet manager
///
/// Coordinates all wallet operations by integrating config, storage, Bitcoin, and F1r3fly layers.
pub struct WalletManager {
    /// Global configuration
    config: GlobalConfig,

    /// Currently loaded Bitcoin wallet (if any)
    bitcoin_wallet: Option<BitcoinWallet>,

    /// Currently loaded wallet metadata (if any)
    wallet_metadata: Option<WalletMetadata>,

    /// Currently loaded wallet keys (decrypted, for F1r3fly operations)
    ///
    /// These keys are kept in memory while a wallet is loaded to avoid
    /// repeated decryption. Cleared when wallet is closed or manager is dropped.
    /// Includes both Bitcoin and F1r3fly keys derived from the mnemonic.
    loaded_wallet_keys: Option<WalletKeys>,

    /// F1r3fly executor manager (initialized when wallet is loaded)
    f1r3fly_executor: Option<F1r3flyExecutorManager>,

    /// F1r3fly contracts manager (initialized when wallet is loaded)
    f1r3fly_contracts: Option<F1r3flyContractsManager>,

    /// Esplora client for blockchain interaction
    esplora_client: EsploraClient,

    /// Set of UTXOs marked as RGB-occupied
    rgb_occupied: HashSet<OutPoint>,
}

impl WalletManager {
    /// Create a new wallet manager
    ///
    /// Loads the global configuration and initializes the Esplora client.
    ///
    /// # Arguments
    ///
    /// * `config` - Global configuration
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::manager::WalletManager;
    /// use f1r3fly_rgb_wallet::config::GlobalConfig;
    ///
    /// let config = GlobalConfig::default_regtest();
    /// let manager = WalletManager::new(config)?;
    /// ```
    pub fn new(config: GlobalConfig) -> Result<Self, ManagerError> {
        // Create Esplora client from config
        let esplora_client =
            EsploraClient::new(&config.bitcoin.esplora_url, config.bitcoin.network)?;

        Ok(Self {
            config,
            bitcoin_wallet: None,
            wallet_metadata: None,
            loaded_wallet_keys: None,
            f1r3fly_executor: None,
            f1r3fly_contracts: None,
            esplora_client,
            rgb_occupied: HashSet::new(),
        })
    }

    /// Create a new wallet
    ///
    /// Generates a new mnemonic, derives keys, creates the wallet directory,
    /// saves encrypted keys, and initializes the BDK wallet.
    ///
    /// # Arguments
    ///
    /// * `name` - Wallet name
    /// * `password` - Password for key encryption
    ///
    /// # Returns
    ///
    /// The generated mnemonic (for user backup)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mnemonic = manager.create_wallet("my-wallet", "password123")?;
    /// println!("Backup this mnemonic: {}", mnemonic);
    /// ```
    pub fn create_wallet(&mut self, name: &str, password: &str) -> Result<String, ManagerError> {
        // Create wallet directory (will error if already exists)
        let wallets_dir = self.config.wallets_dir.as_deref();
        create_wallet_directory(name, wallets_dir).map_err(|e| match e {
            FileSystemError::WalletExists(name) => ManagerError::WalletAlreadyExists(name),
            other => other.into(),
        })?;

        // Generate mnemonic and derive keys
        let mnemonic = generate_mnemonic()?;
        let wallet_keys = WalletKeys::from_mnemonic(&mnemonic, self.config.bitcoin.network)?;

        // Create metadata
        let metadata = WalletMetadata::new(name.to_string(), self.config.bitcoin.network);

        // Save encrypted wallet
        save_wallet(name, &wallet_keys, &metadata, password, wallets_dir)?;

        // Initialize BDK wallet
        let wallet_path = wallet_dir(name, wallets_dir)?;
        let mut bitcoin_wallet = BitcoinWallet::new(
            wallet_keys.bitcoin_descriptor.clone(),
            self.config.bitcoin.network,
            &wallet_path,
        )?;

        // Reveal the first external address so the wallet is ready to receive funds
        // This ensures get_addresses() returns a tracked address that BDK will sync
        use bdk_wallet::KeychainKind;
        bitcoin_wallet
            .inner_mut()
            .reveal_next_address(KeychainKind::External);
        bitcoin_wallet.persist()?;

        // Store in manager
        self.bitcoin_wallet = Some(bitcoin_wallet);
        self.wallet_metadata = Some(metadata);
        self.loaded_wallet_keys = Some(wallet_keys.clone()); // Cache decrypted keys for F1r3fly operations

        // Initialize F1r3fly managers
        self.initialize_f1r3fly(name, &wallet_keys)?;

        // Return mnemonic for user backup
        Ok(mnemonic.to_string())
    }

    /// Import an existing wallet from mnemonic
    ///
    /// Derives keys from the provided mnemonic, creates the wallet directory,
    /// saves encrypted keys, and initializes the BDK wallet.
    ///
    /// # Arguments
    ///
    /// * `name` - Wallet name
    /// * `mnemonic` - BIP39 mnemonic phrase
    /// * `password` - Password for key encryption
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    /// manager.import_wallet("imported-wallet", mnemonic, "password123")?;
    /// ```
    pub fn import_wallet(
        &mut self,
        name: &str,
        mnemonic: &str,
        password: &str,
    ) -> Result<(), ManagerError> {
        // Create wallet directory (will error if already exists)
        let wallets_dir = self.config.wallets_dir.as_deref();
        create_wallet_directory(name, wallets_dir).map_err(|e| match e {
            FileSystemError::WalletExists(name) => ManagerError::WalletAlreadyExists(name),
            other => other.into(),
        })?;

        // Parse and validate mnemonic
        let mnemonic = bip39::Mnemonic::parse(mnemonic)
            .map_err(|e| ManagerError::InvalidMnemonic(e.to_string()))?;

        // Derive keys from mnemonic
        let wallet_keys = WalletKeys::from_mnemonic(&mnemonic, self.config.bitcoin.network)?;

        // Create metadata
        let metadata = WalletMetadata::new(name.to_string(), self.config.bitcoin.network);

        // Save encrypted wallet
        save_wallet(name, &wallet_keys, &metadata, password, wallets_dir)?;

        // Initialize BDK wallet
        let wallet_path = wallet_dir(name, wallets_dir)?;
        let mut bitcoin_wallet = BitcoinWallet::new(
            wallet_keys.bitcoin_descriptor.clone(),
            self.config.bitcoin.network,
            &wallet_path,
        )?;

        // Reveal the first external address so the wallet is ready to receive funds
        // This ensures get_addresses() returns a tracked address that BDK will sync
        use bdk_wallet::KeychainKind;
        bitcoin_wallet
            .inner_mut()
            .reveal_next_address(KeychainKind::External);
        bitcoin_wallet.persist()?;

        // Store in manager
        self.bitcoin_wallet = Some(bitcoin_wallet);
        self.wallet_metadata = Some(metadata);
        self.loaded_wallet_keys = Some(wallet_keys.clone()); // Cache decrypted keys for F1r3fly operations

        // Initialize F1r3fly managers
        self.initialize_f1r3fly(name, &wallet_keys)?;

        Ok(())
    }

    /// Load an existing wallet
    ///
    /// Loads wallet keys and metadata from disk, decrypts keys,
    /// and initializes the BDK wallet.
    ///
    /// # Arguments
    ///
    /// * `name` - Wallet name
    /// * `password` - Password for key decryption
    ///
    /// # Example
    ///
    /// ```ignore
    /// manager.load_wallet("my-wallet", "password123")?;
    /// ```
    pub fn load_wallet(&mut self, name: &str, password: &str) -> Result<(), ManagerError> {
        // Load wallet from disk
        let wallets_dir = self.config.wallets_dir.as_deref();
        let (wallet_keys, metadata) = load_wallet(name, password, wallets_dir)?;

        // Initialize BDK wallet
        let wallet_path = wallet_dir(name, wallets_dir)?;
        let bitcoin_wallet = BitcoinWallet::new(
            wallet_keys.bitcoin_descriptor.clone(),
            self.config.bitcoin.network,
            &wallet_path,
        )?;

        // BDK automatically restores revealed address state from the database
        // No manual intervention needed

        // Store in manager
        self.bitcoin_wallet = Some(bitcoin_wallet);
        self.wallet_metadata = Some(metadata);
        self.loaded_wallet_keys = Some(wallet_keys.clone()); // Cache decrypted keys for F1r3fly operations

        // Initialize F1r3fly managers
        self.initialize_f1r3fly(name, &wallet_keys)?;

        Ok(())
    }

    /// Get reference to currently loaded wallet metadata
    pub fn metadata(&self) -> Option<&WalletMetadata> {
        self.wallet_metadata.as_ref()
    }

    /// Get reference to currently loaded Bitcoin wallet
    pub fn bitcoin_wallet(&self) -> Option<&BitcoinWallet> {
        self.bitcoin_wallet.as_ref()
    }

    /// Get mutable reference to currently loaded Bitcoin wallet
    pub fn bitcoin_wallet_mut(&mut self) -> Option<&mut BitcoinWallet> {
        self.bitcoin_wallet.as_mut()
    }

    /// Get reference to F1r3fly contracts manager
    pub fn f1r3fly_contracts(&self) -> Option<&F1r3flyContractsManager> {
        self.f1r3fly_contracts.as_ref()
    }

    /// Get mutable reference to F1r3fly contracts manager
    pub fn f1r3fly_contracts_mut(&mut self) -> Option<&mut F1r3flyContractsManager> {
        self.f1r3fly_contracts.as_mut()
    }

    /// Check if a wallet is currently loaded
    pub fn is_wallet_loaded(&self) -> bool {
        self.bitcoin_wallet.is_some()
    }

    /// Sync the currently loaded wallet with the blockchain
    ///
    /// # Returns
    ///
    /// Sync result with height and transaction counts
    ///
    /// # Example
    ///
    /// ```ignore
    /// let result = manager.sync_wallet()?;
    /// println!("Synced to height: {}", result.height);
    /// println!("New transactions: {}", result.new_transactions);
    /// ```
    pub fn sync_wallet(&mut self) -> Result<SyncResult, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let result = sync_wallet(wallet, &self.esplora_client)?;

        Ok(result)
    }

    /// Get the balance of the currently loaded wallet
    ///
    /// # Returns
    ///
    /// Balance information (confirmed, unconfirmed, total)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let balance = manager.get_balance()?;
    /// println!("Confirmed: {} sats", balance.confirmed);
    /// println!("Total: {} sats", balance.total);
    /// ```
    pub fn get_balance(&self) -> Result<Balance, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let balance = get_balance(wallet)?;

        Ok(balance)
    }

    /// Get addresses from the currently loaded wallet
    ///
    /// # Arguments
    ///
    /// * `count` - Optional number of addresses to return
    ///
    /// # Returns
    ///
    /// List of address information
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Get all revealed addresses
    /// let addresses = manager.get_addresses(None)?;
    ///
    /// // Get first 10 addresses
    /// let addresses = manager.get_addresses(Some(10))?;
    /// ```
    pub fn get_addresses(&mut self, count: Option<u32>) -> Result<Vec<AddressInfo>, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let addresses = get_addresses(wallet, count)?;

        Ok(addresses)
    }

    /// Get a new receive address
    ///
    /// # Example
    ///
    /// ```ignore
    /// let address = manager.get_new_address()?;
    /// println!("Send Bitcoin to: {}", address);
    /// ```
    pub fn get_new_address(&mut self) -> Result<String, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let address_info = wallet
            .inner_mut()
            .reveal_next_address(KeychainKind::External);

        // Persist the address index increment
        wallet.persist()?;

        Ok(address_info.address.to_string())
    }

    /// Create a new UTXO by self-sending Bitcoin
    ///
    /// # Arguments
    ///
    /// * `amount` - Amount in satoshis
    /// * `fee_rate` - Fee rate configuration
    /// * `mark_rgb` - Whether to mark the UTXO as RGB-occupied
    ///
    /// # Returns
    ///
    /// UTXO operation result with transaction details
    ///
    /// # Example
    ///
    /// ```ignore
    /// let fee_rate = FeeRateConfig::medium_priority();
    /// let result = manager.create_utxo(10_000, &fee_rate, true)?;
    /// println!("Created UTXO: {}", result.outpoint_id());
    /// ```
    pub fn create_utxo(
        &mut self,
        amount: u64,
        fee_rate: &FeeRateConfig,
        mark_rgb: bool,
    ) -> Result<UtxoOperationResult, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        // Always pass rgb_occupied set to exclude them from coin selection
        let result = create_utxo(
            wallet,
            &self.esplora_client,
            amount,
            fee_rate,
            Some(&mut self.rgb_occupied),
            mark_rgb,
        )?;

        Ok(result)
    }

    /// Send Bitcoin to an address
    ///
    /// # Arguments
    ///
    /// * `address` - Destination Bitcoin address
    /// * `amount` - Amount in satoshis
    /// * `fee_rate` - Fee rate configuration
    ///
    /// # Returns
    ///
    /// Transaction ID
    ///
    /// # Example
    ///
    /// ```ignore
    /// let address = "bcrt1q...";
    /// let amount = 50_000; // 50,000 sats
    /// let fee_rate = FeeRateConfig::medium_priority();
    /// let txid = manager.send_bitcoin(address, amount, &fee_rate)?;
    /// println!("Sent! TXID: {}", txid);
    /// ```
    pub fn send_bitcoin(
        &mut self,
        address: &str,
        amount: u64,
        fee_rate: &FeeRateConfig,
    ) -> Result<String, ManagerError> {
        let wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        // Parse the destination address
        let dest_address = address
            .parse::<bdk_wallet::bitcoin::Address<bdk_wallet::bitcoin::address::NetworkUnchecked>>()
            .map_err(|e| {
                ManagerError::Utxo(UtxoError::InvalidAmount(format!("Invalid address: {}", e)))
            })?
            .assume_checked();

        // CRITICAL SAFETY: Check if any UTXOs are RGB-occupied before sending
        if !self.rgb_occupied.is_empty() {
            log::warn!(
                "⚠️  WARNING: Wallet contains {} RGB-occupied UTXO(s). These will be protected from spending.",
                self.rgb_occupied.len()
            );
            for outpoint in &self.rgb_occupied {
                log::debug!("  Protected RGB UTXO: {}:{}", outpoint.txid, outpoint.vout);
            }
        }

        // Build transaction
        let mut tx_builder = wallet.inner_mut().build_tx();
        tx_builder.add_recipient(
            dest_address.script_pubkey(),
            bdk_wallet::bitcoin::Amount::from_sat(amount),
        );
        tx_builder.fee_rate(fee_rate.to_bdk_fee_rate());

        // CRITICAL SAFETY: Exclude RGB-occupied UTXOs from coin selection
        // This prevents accidental spending of UTXOs that hold RGB assets
        for occupied_outpoint in &self.rgb_occupied {
            tx_builder.add_unspendable(*occupied_outpoint);
        }

        // Finish building
        let mut psbt = tx_builder.finish().map_err(|e| {
            ManagerError::Utxo(UtxoError::BuildFailed(format!(
                "Failed to build transaction: {}",
                e
            )))
        })?;

        // Sign
        #[allow(deprecated)]
        wallet
            .inner_mut()
            .sign(&mut psbt, SignOptions::default())
            .map_err(|e| {
                ManagerError::Utxo(UtxoError::SignFailed(format!("Failed to sign: {}", e)))
            })?;

        // Extract and broadcast
        let tx = psbt.extract_tx().map_err(|e| {
            ManagerError::Utxo(UtxoError::BuildFailed(format!(
                "Failed to extract tx: {}",
                e
            )))
        })?;

        self.esplora_client.inner().broadcast(&tx).map_err(|e| {
            ManagerError::Utxo(UtxoError::BroadcastFailed(format!(
                "Failed to broadcast: {}",
                e
            )))
        })?;

        // Persist wallet changes
        wallet.persist()?;

        Ok(tx.compute_txid().to_string())
    }

    /// Get the set of RGB-occupied outpoints
    pub fn rgb_occupied(&self) -> &HashSet<OutPoint> {
        &self.rgb_occupied
    }

    /// Get mutable reference to RGB-occupied outpoints
    pub fn rgb_occupied_mut(&mut self) -> &mut HashSet<OutPoint> {
        &mut self.rgb_occupied
    }

    /// Set F1r3fly contract derivation index for test isolation
    ///
    /// Sets the starting derivation index for contract key derivation.
    /// This is primarily used by tests to ensure parallel test runs don't
    /// deploy contracts to the same registry URI.
    ///
    /// # Arguments
    ///
    /// * `index` - Starting derivation index
    ///
    /// # Errors
    ///
    /// Returns error if F1r3fly managers are not initialized
    ///
    /// # Example
    ///
    /// ```ignore
    /// // In tests: use test-specific index for isolation
    /// manager.set_f1r3fly_derivation_index(12345)?;
    /// ```
    pub fn set_f1r3fly_derivation_index(&mut self, index: u32) -> Result<(), ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        contracts_manager
            .contracts_mut()
            .executor_mut()
            .set_derivation_index(index);

        Ok(())
    }

    // ========================================================================
    // F1r3fly-RGB Asset Management
    // ========================================================================

    /// Initialize F1r3fly managers
    ///
    /// Creates F1r3flyExecutorManager and F1r3flyContractsManager for the loaded wallet.
    /// This is called internally when a wallet is loaded/created/imported.
    ///
    /// # Arguments
    ///
    /// * `wallet_name` - Name of the wallet
    /// * `wallet_keys` - Decrypted wallet keys (includes F1r3fly private key)
    fn initialize_f1r3fly(
        &mut self,
        wallet_name: &str,
        wallet_keys: &WalletKeys,
    ) -> Result<(), ManagerError> {
        // Create executor manager with F1r3fly key
        let executor_manager = F1r3flyExecutorManager::new(&self.config, wallet_keys)?;

        // Get wallet directory
        let wallets_dir = self.config.wallets_dir.as_deref();
        let wallet_path = wallet_dir(wallet_name, wallets_dir)?;

        // Create or load contracts manager
        let contracts_manager =
            if F1r3flyContractsManager::get_state_file_path(&wallet_path).exists() {
                // Load existing state
                F1r3flyContractsManager::load(&executor_manager, &wallet_path)?
            } else {
                // Create new state
                F1r3flyContractsManager::new(&executor_manager, &wallet_path)?
            };

        // Store in manager
        self.f1r3fly_executor = Some(executor_manager);
        self.f1r3fly_contracts = Some(contracts_manager);

        Ok(())
    }

    /// Issue a new RGB asset
    ///
    /// Creates a new fungible token by deploying a RHO20 contract to F1r3node.
    ///
    /// # Arguments
    ///
    /// * `request` - Asset issuance parameters (ticker, name, supply, precision, genesis UTXO)
    ///
    /// # Returns
    ///
    /// `AssetInfo` with contract ID, verified metadata, and genesis seal
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet is not loaded
    /// - F1r3fly managers are not initialized
    /// - Genesis UTXO doesn't exist in Bitcoin wallet
    /// - Genesis UTXO is already RGB-occupied
    /// - Contract deployment fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = IssueAssetRequest {
    ///     ticker: "TEST".to_string(),
    ///     name: "Test Token".to_string(),
    ///     supply: 1000,
    ///     precision: 0,
    ///     genesis_utxo: "txid:vout".to_string(),
    /// };
    /// let asset_info = manager.issue_asset(request).await?;
    /// ```
    pub async fn issue_asset(
        &mut self,
        request: IssueAssetRequest,
    ) -> Result<AssetInfo, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        // Store genesis_utxo before moving request
        let genesis_utxo_str = request.genesis_utxo.clone();

        let asset_info = issue_asset(contracts_manager, bitcoin_wallet, request).await?;

        // CRITICAL: Mark genesis UTXO as RGB-occupied to prevent accidental spending
        // Parse genesis UTXO format: "txid:vout"
        let parts: Vec<&str> = genesis_utxo_str.split(':').collect();
        if parts.len() != 2 {
            return Err(ManagerError::Asset(crate::f1r3fly::asset::AssetError::DeploymentFailed(
                format!(
                    "Invalid genesis UTXO format '{}' after issuance - cannot protect from spending",
                    genesis_utxo_str
                )
            )));
        }

        let txid = parts[0].parse::<bdk_wallet::bitcoin::Txid>().map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::asset::AssetError::DeploymentFailed(format!(
                "Failed to parse genesis UTXO txid '{}' after issuance - cannot protect from spending: {}",
                genesis_utxo_str, e
            )))
        })?;

        let vout = parts[1].parse::<u32>().map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::asset::AssetError::DeploymentFailed(format!(
                "Failed to parse genesis UTXO vout '{}' after issuance - cannot protect from spending: {}",
                genesis_utxo_str, e
            )))
        })?;

        let outpoint = bdk_wallet::bitcoin::OutPoint { txid, vout };
        self.rgb_occupied.insert(outpoint);
        log::info!(
            "Marked genesis UTXO as RGB-occupied: {}:{} (contract: {})",
            txid,
            vout,
            asset_info.contract_id
        );

        // Persist state to disk after issuing asset
        contracts_manager.save_state()?;

        Ok(asset_info)
    }

    /// List all issued RGB assets
    ///
    /// Returns a list of all assets with their basic metadata (ticker, name, registry URI).
    ///
    /// # Returns
    ///
    /// Vector of `AssetListItem` (only includes assets with complete metadata)
    ///
    /// # Errors
    ///
    /// Returns error if F1r3fly managers are not initialized
    ///
    /// # Example
    ///
    /// ```ignore
    /// let assets = manager.list_assets()?;
    /// for asset in assets {
    ///     println!("{}: {}", asset.ticker, asset.name);
    /// }
    /// ```
    pub fn list_assets(&self) -> Result<Vec<AssetListItem>, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_ref()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        Ok(list_assets(contracts_manager))
    }

    /// Get detailed information for a specific asset
    ///
    /// Retrieves full metadata for an asset, including ticker, name, supply, precision,
    /// genesis seal, and registry URI.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID as string
    ///
    /// # Returns
    ///
    /// `AssetInfo` with complete asset details
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - F1r3fly managers are not initialized
    /// - Contract ID is invalid
    /// - Asset not found
    /// - Genesis UTXO info not found
    ///
    /// # Example
    ///
    /// ```ignore
    /// let asset_info = manager.get_asset_info("contract_id_123")?;
    /// println!("Supply: {}", asset_info.supply);
    /// ```
    pub fn get_asset_info(&self, contract_id: &str) -> Result<AssetInfo, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_ref()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        Ok(get_asset_info(contracts_manager, contract_id)?)
    }

    /// Get RGB balance for all assets
    ///
    /// Queries F1r3node contract state for each asset and returns per-asset
    /// and per-UTXO balance breakdown.
    ///
    /// # Returns
    ///
    /// Vector of `AssetBalance` (one per asset with non-zero balance)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - F1r3fly managers not initialized
    /// - Contract queries fail
    ///
    /// # Example
    ///
    /// ```ignore
    /// let balances = manager.get_rgb_balance().await?;
    /// for balance in balances {
    ///     println!("{} ({}): {}", balance.name, balance.ticker, balance.total);
    ///     for utxo in balance.utxo_balances {
    ///         println!("  {}: {}", utxo.outpoint, utxo.amount);
    ///     }
    /// }
    /// ```
    pub async fn get_rgb_balance(&mut self) -> Result<Vec<AssetBalance>, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        Ok(get_rgb_balance(contracts_manager, bitcoin_wallet).await?)
    }

    /// Get RGB balance for a specific asset
    ///
    /// Queries F1r3node contract state for a single asset across all wallet UTXOs.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID as string
    ///
    /// # Returns
    ///
    /// `AssetBalance` with total and per-UTXO breakdown
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - F1r3fly managers not initialized
    /// - Contract not found
    /// - Query fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// let balance = manager.get_asset_balance("contract_id_123").await?;
    /// println!("Total: {}", balance.total);
    /// ```
    pub async fn get_asset_balance(
        &mut self,
        contract_id: &str,
    ) -> Result<AssetBalance, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        Ok(get_asset_balance(contracts_manager, bitcoin_wallet, contract_id).await?)
    }

    /// Get list of UTXOs occupied by RGB assets
    ///
    /// Returns all wallet UTXOs that hold RGB tokens with asset information.
    ///
    /// # Returns
    ///
    /// Vector of `RgbOccupiedUtxo` with UTXO details and asset info
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - F1r3fly managers not initialized
    /// - Queries fail
    ///
    /// # Example
    ///
    /// ```ignore
    /// let occupied = manager.get_occupied_utxos().await?;
    /// for utxo in occupied {
    ///     println!("UTXO {} holds {} {}",
    ///         utxo.outpoint,
    ///         utxo.amount.unwrap_or(0),
    ///         utxo.ticker.as_deref().unwrap_or("?")
    ///     );
    /// }
    /// ```
    pub async fn get_occupied_utxos(&mut self) -> Result<Vec<RgbOccupiedUtxo>, ManagerError> {
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        Ok(get_occupied_utxos(contracts_manager, bitcoin_wallet).await?)
    }

    /// List all wallet UTXOs with optional RGB enrichment and filtering
    ///
    /// Returns a comprehensive view of all Bitcoin UTXOs, enriched with RGB asset
    /// information if F1r3fly contracts are loaded. Supports filtering by availability,
    /// RGB occupation, confirmation status, and amount.
    ///
    /// # Process
    ///
    /// 1. Get Bitcoin UTXOs from BDK wallet (Step 2)
    /// 2. If F1r3fly contracts loaded, enrich each UTXO with RGB seal info (Step 3)
    /// 3. Update UTXO status based on RGB occupation (Available → RgbOccupied)
    /// 4. Apply filters (available-only, rgb-only, confirmed-only, min-amount)
    /// 5. Return filtered and enriched UTXO list
    ///
    /// # Arguments
    ///
    /// * `filter` - Filter options (see `UtxoFilter`)
    ///
    /// # Returns
    ///
    /// Vector of `UtxoInfo` with Bitcoin data and optional RGB metadata
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - Bitcoin wallet queries fail
    /// - RGB queries fail critically
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::types::UtxoFilter;
    ///
    /// // List all UTXOs
    /// let all_utxos = manager.list_utxos(UtxoFilter::default()).await?;
    ///
    /// // List only confirmed, available UTXOs with at least 0.0003 BTC
    /// let filter = UtxoFilter {
    ///     confirmed_only: true,
    ///     available_only: true,
    ///     min_amount_sats: Some(30_000),
    ///     ..Default::default()
    /// };
    /// let filtered = manager.list_utxos(filter).await?;
    ///
    /// for utxo in filtered {
    ///     println!("{}: {} BTC (status: {})",
    ///         utxo.outpoint,
    ///         utxo.amount_btc,
    ///         utxo.status
    ///     );
    ///     for asset in utxo.rgb_assets {
    ///         println!("  - {} {}", asset.amount.unwrap_or(0), asset.ticker);
    ///     }
    /// }
    /// ```
    pub async fn list_utxos(&mut self, filter: UtxoFilter) -> Result<Vec<UtxoInfo>, ManagerError> {
        // 1. Get Bitcoin UTXOs from wallet (Bitcoin-only data)
        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let mut utxos = bitcoin_wallet.list_all_utxos()?;

        // 2. If F1r3fly contracts loaded, enrich with RGB data
        if let Some(ref mut contracts_manager) = self.f1r3fly_contracts {
            for utxo in &mut utxos {
                // Query RGB seal info for this UTXO
                match get_rgb_seal_info(contracts_manager, &utxo.outpoint).await {
                    Ok(rgb_assets) if !rgb_assets.is_empty() => {
                        // This UTXO holds RGB assets
                        utxo.status = UtxoStatus::RgbOccupied;
                        utxo.rgb_assets = rgb_assets;
                    }
                    Ok(_) => {
                        // No RGB assets on this UTXO (empty vector)
                        // Keep status as Available or Unconfirmed
                    }
                    Err(e) => {
                        // Query failed - log but don't fail the entire operation
                        log::warn!(
                            "Failed to query RGB seal info for UTXO {}: {}",
                            utxo.outpoint,
                            e
                        );
                    }
                }
            }
        }

        // 3. Apply filters
        let filtered_utxos = apply_utxo_filters(utxos, filter);

        Ok(filtered_utxos)
    }

    /// Generate RGB invoice with recipient's public key
    ///
    /// Generates a standard RGB invoice and includes the recipient's F1r3fly public key
    /// for transfer authorization.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID string
    /// * `amount` - Amount to receive
    ///
    /// # Returns
    ///
    /// `InvoiceWithPubkey` containing invoice string and recipient public key
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - F1r3fly not initialized
    /// - Invoice generation fails
    pub fn generate_invoice_with_pubkey(
        &mut self,
        contract_id: &str,
        amount: u64,
    ) -> Result<crate::f1r3fly::InvoiceWithPubkey, ManagerError> {
        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let contracts_manager = self
            .f1r3fly_contracts
            .as_ref()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        crate::f1r3fly::generate_invoice_with_pubkey(
            bitcoin_wallet,
            contracts_manager,
            contract_id,
            amount,
        )
        .map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::AssetError::F1r3flyRgb(
                f1r3fly_rgb::F1r3flyRgbError::InvalidResponse(format!(
                    "Invoice generation failed: {}",
                    e
                )),
            ))
        })
    }

    /// Send RGB asset transfer
    ///
    /// Executes a complete RGB transfer flow:
    /// 1. Parse invoice and validate
    /// 2. Execute F1r3fly contract transfer
    /// 3. Build and broadcast Bitcoin witness transaction
    /// 4. Create and save consignment
    /// 5. Update wallet state
    ///
    /// # Arguments
    ///
    /// * `invoice_str` - RGB invoice string from recipient
    /// * `recipient_pubkey_hex` - Recipient's F1r3fly public key (for transfer authorization)
    /// * `fee_rate` - Bitcoin transaction fee rate
    ///
    /// # Returns
    ///
    /// `TransferResponse` with transaction ID and consignment details
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Wallet not loaded
    /// - Invoice invalid
    /// - Insufficient balance
    /// - Transaction fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// let invoice = "rgb:...";
    /// let recipient_pubkey = "04f1r3fly...";
    /// let fee_rate = FeeRateConfig::medium_priority();
    /// let response = manager.send_transfer(invoice, recipient_pubkey, &fee_rate).await?;
    /// println!("Transfer sent: {}", response.bitcoin_txid);
    /// println!("Consignment: {}", response.consignment_path.display());
    /// ```
    pub async fn send_transfer(
        &mut self,
        invoice_str: &str,
        recipient_pubkey_hex: String,
        fee_rate: &FeeRateConfig,
    ) -> Result<crate::f1r3fly::TransferResponse, ManagerError> {
        let bitcoin_wallet = self
            .bitcoin_wallet
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::F1r3flyNotInitialized)?;

        // Get wallet directory for consignments
        let wallet_name = self
            .wallet_metadata
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?
            .name
            .clone();

        // Construct wallet directory path
        let wallets_base = if let Some(dir_str) = &self.config.wallets_dir {
            std::path::PathBuf::from(dir_str)
        } else {
            std::path::PathBuf::from(".f1r3fly-rgb-wallet/wallets")
        };

        let wallet_dir = wallets_base.join(&wallet_name);

        // VALIDATE: Wallet directory must exist (wallet is loaded)
        if !wallet_dir.exists() {
            return Err(ManagerError::FileSystem(FileSystemError::WalletNotFound(
                format!(
                    "Wallet directory not found: {}. Wallet state may be corrupted.",
                    wallet_dir.display()
                ),
            )));
        }

        // Consignments subdirectory (will be created if needed during transfer)
        let consignments_dir = wallet_dir.join("consignments");

        // CRITICAL SAFETY: Check if any UTXOs are RGB-occupied
        if !self.rgb_occupied.is_empty() {
            log::warn!(
                "⚠️  Wallet contains {} RGB-occupied UTXO(s). These will be protected during transfer.",
                self.rgb_occupied.len()
            );
        }

        // Execute transfer
        let response = crate::f1r3fly::send_transfer(
            bitcoin_wallet,
            &self.esplora_client,
            contracts_manager,
            invoice_str,
            recipient_pubkey_hex,
            fee_rate,
            consignments_dir,
            &self.rgb_occupied,
        )
        .await
        .map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::AssetError::F1r3flyRgb(
                f1r3fly_rgb::F1r3flyRgbError::InvalidResponse(format!("Transfer failed: {}", e)),
            ))
        })?;

        Ok(response)
    }

    /// Export genesis consignment for an issued asset
    ///
    /// Creates a genesis consignment that can be sent to recipients to enable
    /// them to receive transfers of this asset.
    ///
    /// # Arguments
    ///
    /// * `wallet_name` - Name of the wallet containing the contract
    /// * `contract_id` - Contract ID to export genesis for
    ///
    /// # Returns
    ///
    /// `ExportGenesisResponse` with consignment file path and metadata
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use f1r3fly_rgb_wallet::manager::WalletManager;
    /// # async fn example(manager: &mut WalletManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let response = manager.export_genesis(
    ///     "my-wallet",
    ///     "contract_abc123...",
    /// ).await?;
    ///
    /// println!("Genesis exported: {}", response.consignment_path.display());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn export_genesis(
        &mut self,
        contract_id: &str,
    ) -> Result<crate::f1r3fly::ExportGenesisResponse, ManagerError> {
        // Wallet must be loaded
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        let metadata = self
            .wallet_metadata
            .as_ref()
            .ok_or(ManagerError::WalletNotLoaded)?;

        // Get wallet directory for consignments
        let wallets_base = if let Some(dir_str) = &self.config.wallets_dir {
            std::path::PathBuf::from(dir_str)
        } else {
            std::path::PathBuf::from(".f1r3fly-rgb-wallet/wallets")
        };

        let wallet_dir = wallets_base.join(&metadata.name);

        // VALIDATE: Wallet directory must exist
        if !wallet_dir.exists() {
            return Err(ManagerError::WalletNotLoaded);
        }

        // Consignments subdirectory
        let consignments_dir = wallet_dir.join("consignments");

        // Export genesis
        let response = crate::f1r3fly::export_genesis(
            contracts_manager,
            &self.esplora_client,
            contract_id,
            consignments_dir,
        )
        .await
        .map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::AssetError::F1r3flyRgb(
                f1r3fly_rgb::F1r3flyRgbError::InvalidResponse(format!(
                    "Export genesis failed: {}",
                    e
                )),
            ))
        })?;

        Ok(response)
    }

    /// Accept received consignment
    ///
    /// Validates and imports a consignment from another party, enabling the wallet
    /// to receive transfers of the asset.
    ///
    /// Performs:
    /// - F1r3node block finalization check
    /// - Tapret proof verification
    /// - Seal validation
    /// - Contract import
    /// - State persistence
    ///
    /// # Arguments
    ///
    /// * `wallet_name` - Name of the wallet to import into
    /// * `consignment_path` - Path to consignment file
    ///
    /// # Returns
    ///
    /// `AcceptConsignmentResponse` with imported contract details
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use f1r3fly_rgb_wallet::manager::WalletManager;
    /// # async fn example(manager: &mut WalletManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let response = manager.accept_consignment(
    ///     "my-wallet",
    ///     "/path/to/consignment.json",
    /// ).await?;
    ///
    /// println!("Accepted: {} {}", response.ticker, response.name);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_consignment(
        &mut self,
        consignment_path: &str,
    ) -> Result<crate::f1r3fly::AcceptConsignmentResponse, ManagerError> {
        // Wallet must be loaded
        let contracts_manager = self
            .f1r3fly_contracts
            .as_mut()
            .ok_or(ManagerError::WalletNotLoaded)?;

        // Accept consignment
        let response = crate::f1r3fly::accept_consignment(
            contracts_manager,
            std::path::Path::new(consignment_path),
        )
        .await
        .map_err(|e| {
            ManagerError::Asset(crate::f1r3fly::AssetError::F1r3flyRgb(
                f1r3fly_rgb::F1r3flyRgbError::InvalidResponse(format!(
                    "Accept consignment failed: {}",
                    e
                )),
            ))
        })?;

        Ok(response)
    }
}

/// Apply filters to a list of UTXOs
///
/// Filters UTXOs based on the provided criteria:
/// - `available_only`: Only include non-RGB UTXOs
/// - `rgb_only`: Only include RGB-occupied UTXOs
/// - `confirmed_only`: Only include confirmed UTXOs
/// - `min_amount_sats`: Only include UTXOs with at least this amount
///
/// # Arguments
///
/// * `utxos` - Vector of UTXOs to filter
/// * `filter` - Filter criteria
///
/// # Returns
///
/// Filtered vector of UTXOs
fn apply_utxo_filters(utxos: Vec<UtxoInfo>, filter: UtxoFilter) -> Vec<UtxoInfo> {
    utxos
        .into_iter()
        .filter(|utxo| {
            // available_only filter: exclude RGB-occupied UTXOs
            if filter.available_only && utxo.status == UtxoStatus::RgbOccupied {
                return false;
            }

            // rgb_only filter: exclude non-RGB UTXOs
            if filter.rgb_only && utxo.status != UtxoStatus::RgbOccupied {
                return false;
            }

            // confirmed_only filter: exclude unconfirmed UTXOs
            if filter.confirmed_only && utxo.confirmations == 0 {
                return false;
            }

            // min_amount filter: exclude UTXOs below threshold
            if let Some(min_amount) = filter.min_amount_sats {
                if utxo.amount_sats < min_amount {
                    return false;
                }
            }

            true
        })
        .collect()
}
