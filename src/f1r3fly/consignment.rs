//! Consignment operations for F1r3fly-RGB wallet
//!
//! Handles exporting genesis consignments and accepting received consignments.

use std::path::{Path, PathBuf};

use crate::bitcoin::network::{EsploraClient, NetworkError};
use crate::f1r3fly::{AssetError, F1r3flyContractsManager};

use amplify::confinement::SmallOrdMap;
use bp::seals::{Noise, TxoSeal, TxoSealExt, WOutpoint, WTxoSeal};
use bp::{Outpoint, Txid};
use std::str::FromStr;
use strict_types::StrictDumb;

/// Error type for consignment operations
#[derive(Debug, thiserror::Error)]
pub enum ConsignmentError {
    /// Contract not found
    #[error("Contract not found: {0}")]
    ContractNotFound(String),

    /// Genesis UTXO not found
    #[error("Genesis UTXO not found for contract: {0}")]
    GenesisNotFound(String),

    /// Genesis execution data not found
    #[error("Genesis execution data not found for contract: {0}. Asset may have been issued with an older wallet version.")]
    GenesisExecutionDataMissing(String),

    /// Invalid consignment
    #[error("Invalid consignment: {0}")]
    Invalid(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    /// F1r3fly RGB error
    #[error("F1r3fly RGB error: {0}")]
    F1r3flyRgb(#[from] f1r3fly_rgb::F1r3flyRgbError),

    /// Tapret error
    #[error("Tapret error: {0}")]
    Tapret(#[from] f1r3fly_rgb::TapretError),

    /// Asset error
    #[error("Asset error: {0}")]
    Asset(#[from] AssetError),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Response from exporting genesis consignment
#[derive(Debug, Clone)]
pub struct ExportGenesisResponse {
    /// Contract ID
    pub contract_id: String,

    /// Consignment file path
    pub consignment_path: PathBuf,

    /// Consignment size in bytes
    pub consignment_size: usize,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,
}

/// Response from accepting consignment
#[derive(Debug, Clone)]
pub struct AcceptConsignmentResponse {
    /// Contract ID
    pub contract_id: String,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,

    /// Number of seals imported
    pub seals_imported: usize,

    /// Genesis UTXO txid
    pub genesis_txid: String,

    /// Genesis UTXO vout
    pub genesis_vout: u32,
}

/// Export genesis consignment
///
/// Creates a genesis consignment for an issued asset, containing:
/// - Contract metadata (Rholang source, methods)
/// - Genesis operation proof
/// - Bitcoin anchor
/// - Genesis seal
///
/// This consignment can be sent to recipients to enable them to accept
/// transfers of this asset.
///
/// # Arguments
///
/// * `contracts_manager` - Contracts manager with the contract
/// * `esplora_client` - Esplora client for fetching Bitcoin transactions
/// * `contract_id_str` - Contract ID to export
/// * `consignments_dir` - Directory to save consignment
///
/// # Returns
///
/// `ExportGenesisResponse` with file path and metadata
pub async fn export_genesis(
    contracts_manager: &mut F1r3flyContractsManager,
    esplora_client: &EsploraClient,
    contract_id_str: &str,
    consignments_dir: PathBuf,
) -> Result<ExportGenesisResponse, ConsignmentError> {
    log::info!(
        "📦 Exporting genesis consignment for contract: {}",
        contract_id_str
    );

    // Parse contract ID
    let contract_id = hypersonic::ContractId::from_str(contract_id_str)
        .map_err(|e| ConsignmentError::Invalid(format!("Invalid contract ID: {:?}", e)))?;

    // Get genesis UTXO info
    let genesis_info = contracts_manager
        .get_genesis_utxo(contract_id_str)
        .ok_or_else(|| ConsignmentError::GenesisNotFound(contract_id_str.to_string()))?
        .clone();

    // Get contract
    let contract = contracts_manager
        .contracts_mut()
        .get_mut(&contract_id)
        .ok_or_else(|| ConsignmentError::ContractNotFound(contract_id_str.to_string()))?;

    log::debug!("  Ticker: {}", genesis_info.ticker);
    log::debug!("  Name: {}", genesis_info.name);
    log::debug!(
        "  Genesis UTXO: {}:{}",
        genesis_info.txid,
        genesis_info.vout
    );

    // Get genesis execution data (from issue operation)
    let genesis_exec_data = genesis_info
        .genesis_execution_result
        .as_ref()
        .ok_or_else(|| {
            ConsignmentError::GenesisExecutionDataMissing(contract_id_str.to_string())
        })?;

    log::debug!(
        "  Genesis state hash: {}",
        hex::encode(genesis_exec_data.state_hash)
    );
    log::debug!(
        "  Genesis block: {}",
        genesis_exec_data.finalized_block_hash
    );

    // Create genesis seal from genesis UTXO
    let genesis_txid = Txid::from_str(&genesis_info.txid)
        .map_err(|e| ConsignmentError::Invalid(format!("Invalid genesis txid: {}", e)))?;
    let genesis_outpoint = Outpoint::new(genesis_txid, genesis_info.vout);
    let genesis_seal = TxoSeal {
        primary: genesis_outpoint,
        secondary: TxoSealExt::Noise(Noise::strict_dumb()),
    };

    // Query the contract balance to verify state
    let balance = contract.balance(&genesis_seal).await?;
    log::debug!("  Genesis balance: {}", balance);

    // Reconstruct F1r3flyExecutionResult from stored genesis data
    use amplify::confinement::SmallVec;
    let opid_bytes = hex::decode(&genesis_exec_data.opid)
        .map_err(|e| ConsignmentError::Invalid(format!("Invalid opid hex: {}", e)))?;
    if opid_bytes.len() != 32 {
        return Err(ConsignmentError::Invalid(format!(
            "Invalid opid length: expected 32 bytes, got {}",
            opid_bytes.len()
        )));
    }
    let mut opid_array = [0u8; 32];
    opid_array.copy_from_slice(&opid_bytes);

    let genesis_result = f1r3fly_rgb::F1r3flyExecutionResult {
        opid: rgb::Opid::from(opid_array),
        deploy_id: SmallVec::try_from(genesis_exec_data.deploy_id.as_bytes().to_vec()).map_err(
            |e| ConsignmentError::Invalid(format!("Failed to create deploy_id: {:?}", e)),
        )?,
        finalized_block_hash: SmallVec::try_from(
            genesis_exec_data.finalized_block_hash.as_bytes().to_vec(),
        )
        .map_err(|e| ConsignmentError::Invalid(format!("Failed to create block_hash: {:?}", e)))?,
        rholang_source: SmallVec::try_from(genesis_exec_data.rholang_source.as_bytes().to_vec())
            .map_err(|e| {
                ConsignmentError::Invalid(format!("Failed to create rholang_source: {:?}", e))
            })?,
        state_hash: genesis_exec_data.state_hash,
    };

    log::debug!("  Genesis execution result reconstructed");

    // Create seals map with genesis seal
    let mut seals_map = SmallOrdMap::new();
    let genesis_wtxo_seal = WTxoSeal {
        primary: WOutpoint::Extern(genesis_seal.primary),
        secondary: genesis_seal.secondary,
    };
    seals_map
        .insert(0u16, genesis_wtxo_seal)
        .map_err(|_| ConsignmentError::Invalid("Failed to insert genesis seal".to_string()))?;

    // Fetch the actual Bitcoin transaction that contains the genesis UTXO
    // This transaction must have the Tapret commitment embedded for validation
    log::debug!(
        "  Fetching witness transaction from blockchain: {}",
        genesis_info.txid
    );

    // Parse txid for Esplora query
    use bdk_wallet::bitcoin::Txid as BdkTxid;
    let bdk_txid = BdkTxid::from_str(&genesis_info.txid)
        .map_err(|e| ConsignmentError::Invalid(format!("Invalid txid: {}", e)))?;

    let bdk_tx = esplora_client
        .inner()
        .get_tx(&bdk_txid)
        .map_err(|e| ConsignmentError::Network(NetworkError::Esplora(e)))?
        .ok_or_else(|| {
            ConsignmentError::Invalid(format!(
                "Transaction {} not found on blockchain",
                genesis_info.txid
            ))
        })?;

    // Convert BDK transaction to bp::Tx for consignment
    use bp::ConsensusDecode;
    let tx_bytes = bdk_wallet::bitcoin::consensus::encode::serialize(&bdk_tx);
    let bc_tx = bpstd::Tx::consensus_deserialize(&tx_bytes[..]).map_err(|e| {
        ConsignmentError::Invalid(format!("Failed to deserialize witness TX: {:?}", e))
    })?;
    let witness_tx: bp::Tx = bc_tx.into();

    log::debug!("  Witness transaction fetched and converted");

    // Create genesis consignment with REAL genesis Bitcoin transaction
    // For genesis: is_genesis=true tells the consignment constructor to:
    //   - Create a placeholder anchor (no Tapret proof needed)
    //   - Skip Tapret validation during acceptance
    // This follows RGB protocol: genesis UTXO itself is the Bitcoin anchor
    let consignment = f1r3fly_rgb::F1r3flyConsignment::new(
        contract,
        genesis_result,
        seals_map,
        vec![witness_tx], // Real Bitcoin TX containing genesis UTXO
        true,             // is_genesis - creates placeholder anchor, skips Tapret validation
    )?;

    log::info!("✓ Genesis consignment created");

    // Serialize and save
    let consignment_bytes = consignment.to_bytes()?;
    let consignment_filename = format!("{}_genesis.json", contract_id_str);

    std::fs::create_dir_all(&consignments_dir)?;
    let consignment_path = consignments_dir.join(&consignment_filename);
    std::fs::write(&consignment_path, &consignment_bytes)?;

    log::info!("✓ Genesis consignment saved");
    log::debug!("  Path: {}", consignment_path.display());
    log::debug!("  Size: {} bytes", consignment_bytes.len());

    Ok(ExportGenesisResponse {
        contract_id: contract_id_str.to_string(),
        consignment_path,
        consignment_size: consignment_bytes.len(),
        ticker: genesis_info.ticker,
        name: genesis_info.name,
    })
}

/// Accept received consignment
///
/// Validates and imports a consignment received from another party.
/// This enables the wallet to:
/// - Receive transfers of the asset
/// - Query balances
/// - Create invoices for this asset
///
/// Validation includes:
/// - F1r3node block finalization check
/// - Tapret proof verification (Bitcoin anchor)
/// - Seal validation
///
/// # Arguments
///
/// * `contracts_manager` - Contracts manager to import into
/// * `consignment_path` - Path to consignment file
///
/// # Returns
///
/// `AcceptConsignmentResponse` with imported contract details
pub async fn accept_consignment(
    contracts_manager: &mut F1r3flyContractsManager,
    consignment_path: &Path,
) -> Result<AcceptConsignmentResponse, ConsignmentError> {
    log::info!(
        "📥 Accepting consignment from: {}",
        consignment_path.display()
    );

    // Load consignment from file
    let consignment_bytes = std::fs::read(consignment_path)?;
    let consignment = f1r3fly_rgb::F1r3flyConsignment::from_bytes(&consignment_bytes)?;

    log::debug!("  Contract ID: {}", consignment.contract_id());
    log::debug!("  Seals: {}", consignment.seals().len());

    // Validate consignment
    log::info!("🔍 Validating consignment...");

    // Get executor from contracts for validation
    let executor = contracts_manager.contracts().executor();
    consignment.validate(executor).await?;

    log::info!("✓ Consignment validated");

    // Extract contract metadata
    let contract_id = consignment.contract_id();
    let metadata = consignment.metadata();
    let contract_id_str = contract_id.to_string();

    log::debug!("  Registry URI: {}", metadata.registry_uri);

    // Check if contract already exists
    let contract_exists = contracts_manager.contracts().get(&contract_id).is_some();

    if consignment.is_genesis {
        // Genesis consignments should NOT have existing contract
        if contract_exists {
            log::warn!("⚠️  Genesis consignment for existing contract");
            return Err(ConsignmentError::Invalid(format!(
                "Genesis consignment for contract {} but contract already exists in wallet",
                contract_id_str
            )));
        }
    } else {
        // Transfer consignments MUST have existing contract
        if !contract_exists {
            log::warn!("⚠️  Transfer consignment for unknown contract");
            return Err(ConsignmentError::Invalid(format!(
                "Transfer consignment for contract {} but contract does not exist in wallet",
                contract_id_str
            )));
        }

        // For transfers, contract already exists - just verify and return
        // State is tracked on F1r3fly blockchain, not locally
        log::info!("✓ Transfer consignment accepted for existing contract");

        let genesis_info = contracts_manager
            .genesis_utxos()
            .get(&contract_id_str)
            .ok_or_else(|| {
                ConsignmentError::Invalid(format!(
                    "No genesis UTXO info for contract {}",
                    contract_id_str
                ))
            })?;

        return Ok(AcceptConsignmentResponse {
            contract_id: contract_id_str.clone(),
            ticker: genesis_info.ticker.clone(),
            name: genesis_info.name.clone(),
            seals_imported: 1, // Transfer consignment has 1 seal (recipient)
            genesis_txid: genesis_info.txid.clone(),
            genesis_vout: genesis_info.vout,
        });
    }

    // Below this point: Genesis consignment only - import contract metadata locally
    // For consignment acceptance, the contract execution state is on F1r3node
    // We register the genesis UTXO info so we can query balances and create invoices
    log::info!("📝 Importing genesis UTXO info...");

    // Register anchors in tracker
    let f1r3fly_proof = consignment.f1r3fly_proof();
    let opid_bytes: [u8; 32] = f1r3fly_proof.state_hash;
    let opid = rgb::Opid::from(opid_bytes);

    contracts_manager
        .tracker_mut()
        .add_anchor(opid, consignment.bitcoin_anchor.clone());
    log::debug!("✓ Anchor registered in tracker");

    // Import genesis UTXO info
    // Extract from first seal in consignment
    let first_seal = consignment
        .seals()
        .iter()
        .next()
        .ok_or_else(|| ConsignmentError::Invalid("Consignment has no seals".to_string()))?
        .1;

    // Extract outpoint from seal
    let outpoint = match &first_seal.primary {
        WOutpoint::Extern(outpoint) => outpoint,
        _ => {
            return Err(ConsignmentError::Invalid(
                "Expected Extern seal".to_string(),
            ))
        }
    };

    // Register the contract with Bob's executor so he can query it
    // This imports the contract metadata from the consignment
    log::debug!("  Registering contract from consignment with Bob's executor...");

    contracts_manager
        .contracts_mut()
        .executor_mut()
        .register_contract(contract_id, consignment.contract_metadata.clone());

    log::debug!("✓ Contract registered with executor");

    // Query the contract for asset metadata
    log::debug!("  Querying contract for asset metadata...");

    let metadata_json = contracts_manager
        .contracts()
        .executor()
        .query_state(contract_id, "getMetadata", &[])
        .await
        .map_err(|e| {
            ConsignmentError::Invalid(format!("Failed to query contract metadata: {}", e))
        })?;

    let ticker = metadata_json["ticker"]
        .as_str()
        .ok_or_else(|| {
            ConsignmentError::Invalid(format!(
                "Contract metadata missing 'ticker' field: {:?}",
                metadata_json
            ))
        })?
        .to_string();

    let name = metadata_json["name"]
        .as_str()
        .ok_or_else(|| {
            ConsignmentError::Invalid(format!(
                "Contract metadata missing 'name' field: {:?}",
                metadata_json
            ))
        })?
        .to_string();

    let supply = metadata_json["supply"].as_u64().ok_or_else(|| {
        ConsignmentError::Invalid(format!(
            "Contract metadata missing or invalid 'supply' field: {:?}",
            metadata_json
        ))
    })?;

    let precision = metadata_json["decimals"].as_u64().ok_or_else(|| {
        ConsignmentError::Invalid(format!(
            "Contract metadata missing or invalid 'decimals' field: {:?}",
            metadata_json
        ))
    })? as u8;

    log::debug!(
        "✓ Asset metadata retrieved: {} {} (supply: {}, decimals: {})",
        ticker,
        name,
        supply,
        precision
    );

    // Create GenesisExecutionData from consignment's F1r3fly proof
    let genesis_execution_data = crate::f1r3fly::GenesisExecutionData {
        opid: hex::encode(f1r3fly_proof.state_hash),
        deploy_id: f1r3fly_proof.deploy_id.clone(),
        finalized_block_hash: f1r3fly_proof.block_hash.clone(),
        state_hash: f1r3fly_proof.state_hash,
        rholang_source: metadata.rholang_source.clone(),
    };

    let genesis_utxo_info = crate::f1r3fly::GenesisUtxoInfo {
        contract_id: contract_id_str.clone(),
        txid: outpoint.txid.to_string(),
        vout: outpoint.vout.into_u32(),
        ticker: ticker.clone(),
        name: name.clone(),
        precision,
        supply,
        genesis_execution_result: Some(genesis_execution_data),
    };

    contracts_manager.add_genesis_utxo(genesis_utxo_info);
    log::debug!("✓ Genesis UTXO registered");

    // Create a full F1r3flyRgbContract instance for Bob so he can query balances
    // This uses the registered contract metadata
    log::debug!("  Creating F1r3flyRgbContract instance for Bob...");

    let bob_contract = f1r3fly_rgb::F1r3flyRgbContract::new(
        contract_id,
        contracts_manager.contracts().executor().clone(),
        consignment.contract_metadata.clone(),
    )?;

    // Add to contracts manager
    contracts_manager.contracts_mut().register(bob_contract);
    log::debug!("✓ Contract instance created and added to contracts manager");

    // Persist state
    contracts_manager
        .save_state()
        .map_err(|e| ConsignmentError::Serialization(format!("{}", e)))?;
    log::info!("✓ State persisted");

    log::info!("✅ Consignment accepted");

    Ok(AcceptConsignmentResponse {
        contract_id: contract_id_str,
        ticker,
        name,
        seals_imported: consignment.seals().len(),
        genesis_txid: outpoint.txid.to_string(),
        genesis_vout: outpoint.vout.into_u32(),
    })
}
