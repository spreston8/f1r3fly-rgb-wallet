//! F1r3fly Contracts Management
//!
//! Manages F1r3flyRgbContracts with state persistence to disk.
//! Handles contract metadata, Bitcoin anchor tracking, and derivation state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// Re-exports from f1r3fly-rgb
use f1r3fly_rgb::{BitcoinAnchorTracker, ContractMetadata, F1r3flyRgbContracts, TxoSeal};

use crate::f1r3fly::executor::F1r3flyExecutorManager;

/// Error type for contracts manager operations
#[derive(Debug, thiserror::Error)]
pub enum ContractsManagerError {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// State file not found
    #[error("State file not found: {0}")]
    StateNotFound(String),

    /// Invalid state
    #[error("Invalid state: {0}")]
    InvalidState(String),
}

/// Genesis UTXO information for an asset
///
/// Stores the Bitcoin UTXO that received the initial token allocation.
/// This is used during transfer operations to properly register seals with the tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisUtxoInfo {
    /// Contract ID (for reference)
    pub contract_id: String,

    /// Bitcoin transaction ID
    pub txid: String,

    /// Output index
    pub vout: u32,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,

    /// Total supply
    pub supply: u64,

    /// Decimal precision
    pub precision: u8,

    /// F1r3fly execution result from genesis (issue operation)
    /// Required for creating valid genesis consignments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genesis_execution_result: Option<GenesisExecutionData>,
}

/// Serializable subset of F1r3flyExecutionResult for genesis storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisExecutionData {
    /// RGB operation ID
    pub opid: String,

    /// F1r3fly deploy ID
    pub deploy_id: String,

    /// Finalized block hash
    pub finalized_block_hash: String,

    /// State hash (32 bytes)
    pub state_hash: [u8; 32],

    /// Rholang source code
    pub rholang_source: String,
}

/// Persistent state for F1r3fly contracts
///
/// Stores all information needed to recreate the contracts manager:
/// - Derivation index for key derivation
/// - Contract metadata (registry URIs, methods, Rholang source)
/// - Genesis UTXO information for seal registration during transfers
/// - Bitcoin anchor tracker state (seals, witnesses, anchors)
/// - Contract derivation indices for signature generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct F1r3flyState {
    /// Current derivation index for BIP32-style contract key derivation
    pub derivation_index: u32,

    /// Contract metadata by contract ID
    /// Contains registry URIs, available methods, and Rholang source
    pub contracts_metadata: HashMap<String, ContractMetadata>,

    /// Genesis UTXO information indexed by contract ID
    /// Maps contract_id -> genesis UTXO details
    /// Used during transfer operations (Phase 3) to register seals with tracker
    #[serde(default)]
    pub genesis_utxos: HashMap<String, GenesisUtxoInfo>,

    /// Bitcoin anchor tracker state (serialized JSON)
    /// This is stored as a JSON value to avoid complex type parameters in serialization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker_state: Option<serde_json::Value>,

    /// Map of contract ID to derivation index used for deployment
    /// Stores the derivation_index at which each contract was deployed.
    /// This is needed to recover the correct signing key for secured methods like issue().
    #[serde(default)]
    pub contract_derivation_indices: HashMap<String, u32>,
}

impl F1r3flyState {
    /// Create new empty state
    pub fn new() -> Self {
        Self {
            derivation_index: 0,
            contracts_metadata: HashMap::new(),
            genesis_utxos: HashMap::new(),
            tracker_state: None,
            contract_derivation_indices: HashMap::new(),
        }
    }

    /// Check if state is empty (no contracts deployed)
    pub fn is_empty(&self) -> bool {
        self.contracts_metadata.is_empty()
    }
}

impl Default for F1r3flyState {
    fn default() -> Self {
        Self::new()
    }
}

/// Manages F1r3flyRgbContracts with state persistence
///
/// This manager wraps `F1r3flyRgbContracts` and provides automatic state
/// persistence to `f1r3fly_state.json` in the wallet directory.
///
/// # State Management
///
/// - `save_state()` - Saves current state to disk (call after any contract operation)
/// - `load_state()` - Loads state from disk and recreates contracts
/// - State includes: derivation index, contract metadata, Bitcoin anchor tracker
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::f1r3fly::F1r3flyContractsManager;
///
/// // Create new manager
/// let manager = F1r3flyContractsManager::new(executor_manager, wallet_dir)?;
///
/// // Or load existing state
/// let manager = F1r3flyContractsManager::load(executor_manager, wallet_dir)?;
///
/// // Issue asset
/// let contract_id = manager.contracts_mut().issue("BTC", "Bitcoin", 21000000, 8).await?;
///
/// // Save state after operation
/// manager.save_state()?;
/// ```
pub struct F1r3flyContractsManager {
    /// F1r3flyRgbContracts instance (contains executor and contracts HashMap)
    contracts: F1r3flyRgbContracts,

    /// Shared Bitcoin anchor tracker for all contracts
    tracker: BitcoinAnchorTracker<TxoSeal>,

    /// Genesis UTXO information for issued assets
    /// Used during transfer operations (Phase 3) to register seals with tracker
    genesis_utxos: HashMap<String, GenesisUtxoInfo>,

    /// Map of contract ID to derivation index used for deployment
    /// Tracks which derivation index was used to deploy each contract.
    /// This is needed to recover the correct signing key for secured methods like issue().
    contract_derivation_indices: HashMap<String, u32>,

    /// Path to state file (f1r3fly_state.json)
    state_path: PathBuf,
}

impl F1r3flyContractsManager {
    /// Create a new contracts manager with fresh state
    ///
    /// Creates a new `F1r3flyRgbContracts` instance and initializes an empty
    /// Bitcoin anchor tracker. No state file is loaded.
    ///
    /// # Arguments
    ///
    /// * `executor_manager` - Executor manager for creating F1r3flyExecutor
    /// * `wallet_dir` - Wallet directory path (state file will be created here)
    ///
    /// # Returns
    ///
    /// New `F1r3flyContractsManager` with empty state
    ///
    /// # Example
    ///
    /// ```ignore
    /// let manager = F1r3flyContractsManager::new(&executor_manager, &wallet_dir)?;
    /// ```
    pub fn new<P: AsRef<Path>>(
        executor_manager: &F1r3flyExecutorManager,
        wallet_dir: P,
    ) -> Result<Self, ContractsManagerError> {
        let executor = executor_manager.create_executor();
        let contracts = F1r3flyRgbContracts::new(executor);
        let tracker = BitcoinAnchorTracker::new();

        let state_path = wallet_dir.as_ref().join("f1r3fly_state.json");

        Ok(Self {
            contracts,
            tracker,
            genesis_utxos: HashMap::new(),
            contract_derivation_indices: HashMap::new(),
            state_path,
        })
    }

    /// Load contracts manager from existing state file
    ///
    /// Loads `f1r3fly_state.json` from the wallet directory and recreates
    /// the contracts and tracker from saved state.
    ///
    /// # Arguments
    ///
    /// * `executor_manager` - Executor manager for creating F1r3flyExecutor
    /// * `wallet_dir` - Wallet directory path (must contain f1r3fly_state.json)
    ///
    /// # Returns
    ///
    /// Restored `F1r3flyContractsManager` with loaded state
    ///
    /// # Errors
    ///
    /// Returns error if state file doesn't exist or is invalid
    ///
    /// # Example
    ///
    /// ```ignore
    /// let manager = F1r3flyContractsManager::load(&executor_manager, &wallet_dir)?;
    /// ```
    pub fn load<P: AsRef<Path>>(
        executor_manager: &F1r3flyExecutorManager,
        wallet_dir: P,
    ) -> Result<Self, ContractsManagerError> {
        let state_path = wallet_dir.as_ref().join("f1r3fly_state.json");

        // Load state from disk
        let state = Self::load_state_from_file(&state_path)?;

        // Create executor and restore derivation index
        let mut executor = executor_manager.create_executor();
        executor.set_derivation_index(state.derivation_index);

        // Restore contract metadata into executor's cache
        // This is CRITICAL: the executor needs to know about all deployed contracts
        // so that query_state() can find the registry URIs
        use std::str::FromStr;
        for (contract_id_str, metadata) in &state.contracts_metadata {
            let contract_id = f1r3fly_rgb::ContractId::from_str(contract_id_str).map_err(|e| {
                ContractsManagerError::InvalidState(format!("Invalid contract ID in state: {}", e))
            })?;
            executor.register_contract(contract_id, metadata.clone());
        }

        // Create contracts instance
        let mut contracts = F1r3flyRgbContracts::new(executor);

        // Recreate contract instances and add to collection
        // This populates the contracts HashMap so list() returns the loaded contracts
        for (contract_id_str, metadata) in &state.contracts_metadata {
            let contract_id = f1r3fly_rgb::ContractId::from_str(contract_id_str).map_err(|e| {
                ContractsManagerError::InvalidState(format!("Invalid contract ID in state: {}", e))
            })?;

            // Create contract instance with cloned executor
            let contract = f1r3fly_rgb::F1r3flyRgbContract::new(
                contract_id,
                contracts.executor().clone(),
                metadata.clone(),
            )
            .map_err(|e| {
                ContractsManagerError::InvalidState(format!(
                    "Failed to create contract instance: {}",
                    e
                ))
            })?;

            // Add to collection
            contracts.register_loaded_contract(contract_id, contract);
        }

        // Restore tracker state if available
        let tracker = if let Some(tracker_json) = state.tracker_state {
            // Deserialize tracker from JSON
            serde_json::from_value(tracker_json).map_err(|e| {
                ContractsManagerError::InvalidState(format!("Failed to deserialize tracker: {}", e))
            })?
        } else {
            BitcoinAnchorTracker::new()
        };

        Ok(Self {
            contracts,
            tracker,
            genesis_utxos: state.genesis_utxos,
            contract_derivation_indices: state.contract_derivation_indices,
            state_path,
        })
    }

    /// Store the derivation index used for a contract deployment
    ///
    /// This should be called with the index that was ACTUALLY used for deployment.
    /// Since `auto_derive` increments the index during deployment, the caller must
    /// capture the index BEFORE calling the deployment method.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID (as string)
    /// * `derivation_index` - The derivation index that was used for this contract's deployment
    pub fn store_contract_derivation_index(&mut self, contract_id: &str, derivation_index: u32) {
        log::info!(
            "Storing derivation index {} for contract {}",
            derivation_index,
            contract_id
        );
        self.contract_derivation_indices
            .insert(contract_id.to_string(), derivation_index);
    }

    /// Get the derivation index used for a contract's deployment
    ///
    /// Returns the derivation index that was used when the contract was deployed.
    /// This is needed to retrieve the correct signing key for secured methods.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID (as string)
    ///
    /// # Returns
    ///
    /// The derivation index used for this contract
    ///
    /// # Errors
    ///
    /// Returns error if contract not found
    pub fn get_contract_derivation_index(
        &self,
        contract_id: &str,
    ) -> Result<u32, ContractsManagerError> {
        // Load state to get derivation indices
        let state = Self::load_state_from_file(&self.state_path)?;

        // Get derivation index for this contract
        state
            .contract_derivation_indices
            .get(contract_id)
            .copied()
            .ok_or_else(|| {
                ContractsManagerError::InvalidState(format!(
                    "No derivation index found for contract {}",
                    contract_id
                ))
            })
    }

    /// Get the signing key for a contract's secured methods
    ///
    /// Returns the child key that was used to deploy the contract.
    /// This key should be used to sign secured method calls like issue().
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID (as string)
    ///
    /// # Returns
    ///
    /// The secp256k1 secret key for signing
    ///
    /// # Errors
    ///
    /// Returns error if contract not found or key derivation fails
    pub fn get_contract_signing_key(
        &self,
        contract_id: &str,
    ) -> Result<secp256k1::SecretKey, ContractsManagerError> {
        // Load state to get derivation indices
        let state = Self::load_state_from_file(&self.state_path)?;

        // Get derivation index for this contract
        let derivation_index = state
            .contract_derivation_indices
            .get(contract_id)
            .ok_or_else(|| {
                ContractsManagerError::InvalidState(format!(
                    "No derivation index found for contract {}",
                    contract_id
                ))
            })?;

        // Derive child key from executor
        self.contracts
            .executor()
            .get_child_key_at_index(*derivation_index)
            .map_err(|e| {
                ContractsManagerError::InvalidState(format!("Failed to derive signing key: {}", e))
            })
    }

    /// Load state from file helper
    fn load_state_from_file(path: &Path) -> Result<F1r3flyState, ContractsManagerError> {
        if !path.exists() {
            return Err(ContractsManagerError::StateNotFound(
                path.display().to_string(),
            ));
        }

        let json = std::fs::read_to_string(path)?;
        let state: F1r3flyState = serde_json::from_str(&json)?;

        Ok(state)
    }

    /// Save current state to disk
    ///
    /// Serializes the current contracts metadata, derivation index, and Bitcoin
    /// anchor tracker to `f1r3fly_state.json`.
    ///
    /// Should be called after any operation that modifies state:
    /// - Asset issuance
    /// - Transfer execution
    /// - Consignment acceptance
    ///
    /// # Errors
    ///
    /// Returns error if serialization or file write fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// manager.contracts_mut().issue("BTC", "Bitcoin", 21000000, 8).await?;
    /// manager.save_state()?; // Persist changes
    /// ```
    pub fn save_state(&self) -> Result<(), ContractsManagerError> {
        // Extract derivation index from executor
        let derivation_index = self.contracts.executor().derivation_index();

        // Extract contract metadata from executor
        // Convert HashMap<ContractId, ContractMetadata> to HashMap<String, ContractMetadata>
        // for better JSON serialization (ContractId is complex type)
        let contracts_metadata: HashMap<String, ContractMetadata> = self
            .contracts
            .executor()
            .contracts_metadata()
            .iter()
            .map(|(id, metadata)| (id.to_string(), metadata.clone()))
            .collect();

        // Serialize tracker to JSON
        let tracker_state = serde_json::to_value(&self.tracker)
            .map_err(|e| ContractsManagerError::Serialization(e))?;

        // Use the stored derivation indices
        let contract_derivation_indices = self.contract_derivation_indices.clone();

        // Create state object
        let state = F1r3flyState {
            derivation_index,
            contracts_metadata,
            genesis_utxos: self.genesis_utxos.clone(),
            tracker_state: Some(tracker_state),
            contract_derivation_indices,
        };

        // Write to file
        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(&self.state_path, json)?;

        Ok(())
    }

    /// Get reference to F1r3flyRgbContracts
    ///
    /// Use this to query contracts or perform read-only operations.
    pub fn contracts(&self) -> &F1r3flyRgbContracts {
        &self.contracts
    }

    /// Get mutable reference to F1r3flyRgbContracts
    ///
    /// Use this to issue assets, send transfers, or perform write operations.
    /// Remember to call `save_state()` after operations.
    pub fn contracts_mut(&mut self) -> &mut F1r3flyRgbContracts {
        &mut self.contracts
    }

    /// Get reference to Bitcoin anchor tracker
    ///
    /// Use this to query seals, witnesses, and anchors.
    pub fn tracker(&self) -> &BitcoinAnchorTracker<TxoSeal> {
        &self.tracker
    }

    /// Get mutable reference to Bitcoin anchor tracker
    ///
    /// Use this to add seals, witnesses, or anchors.
    /// Remember to call `save_state()` after modifications.
    pub fn tracker_mut(&mut self) -> &mut BitcoinAnchorTracker<TxoSeal> {
        &mut self.tracker
    }

    /// Check if state file exists
    pub fn state_exists(&self) -> bool {
        self.state_path.exists()
    }

    /// Get path to state file
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Get the state file path for a wallet directory (static helper)
    ///
    /// Returns the path where f1r3fly_state.json would be located for the given wallet directory.
    /// This is useful for checking if state exists before loading.
    ///
    /// # Arguments
    ///
    /// * `wallet_dir` - Wallet directory path
    pub fn get_state_file_path<P: AsRef<Path>>(wallet_dir: P) -> std::path::PathBuf {
        wallet_dir.as_ref().join("f1r3fly_state.json")
    }

    /// Add genesis UTXO information for an issued asset
    ///
    /// Stores the genesis UTXO details for future seal registration during transfers.
    /// Remember to call `save_state()` after adding genesis info.
    ///
    /// # Arguments
    ///
    /// * `genesis_info` - Genesis UTXO information to store
    pub fn add_genesis_utxo(&mut self, genesis_info: GenesisUtxoInfo) {
        self.genesis_utxos
            .insert(genesis_info.contract_id.clone(), genesis_info);
    }

    /// Get genesis UTXO information for a contract
    ///
    /// Returns `None` if no genesis UTXO info has been stored for this contract.
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID (as string)
    pub fn get_genesis_utxo(&self, contract_id: &str) -> Option<&GenesisUtxoInfo> {
        self.genesis_utxos.get(contract_id)
    }

    /// Get all genesis UTXO information
    ///
    /// Returns a reference to the HashMap of all stored genesis UTXOs.
    pub fn genesis_utxos(&self) -> &HashMap<String, GenesisUtxoInfo> {
        &self.genesis_utxos
    }
}
