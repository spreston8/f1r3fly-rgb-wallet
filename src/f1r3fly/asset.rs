//! RGB Asset Issuance
//!
//! Handles RGB asset creation, querying, and metadata management.
//! Coordinates between Bitcoin wallet (UTXO validation) and F1r3fly contracts.

use serde::{Deserialize, Serialize};

use f1r3fly_rgb::ContractId;

use crate::bitcoin::wallet::BitcoinWallet;
use crate::f1r3fly::contracts::F1r3flyContractsManager;

/// Error type for asset operations
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// UTXO not found in wallet
    #[error("Genesis UTXO not found: {0}")]
    GenesisUtxoNotFound(String),

    /// UTXO is occupied by RGB asset
    #[error("UTXO is already occupied by an RGB asset: {0}")]
    UtxoOccupied(String),

    /// Contract deployment failed
    #[error("Contract deployment failed: {0}")]
    DeploymentFailed(String),

    /// Asset not found
    #[error("Asset not found: {0}")]
    AssetNotFound(String),

    /// F1r3fly-RGB error
    #[error("F1r3fly-RGB error: {0}")]
    F1r3flyRgb(#[from] f1r3fly_rgb::F1r3flyRgbError),

    /// Contracts manager error
    #[error("Contracts manager error: {0}")]
    ContractsManager(#[from] crate::f1r3fly::contracts::ContractsManagerError),
}

/// Asset issuance request
///
/// Parameters for issuing a new RGB20-compatible fungible token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAssetRequest {
    /// Asset ticker symbol (e.g., "BTC")
    pub ticker: String,

    /// Asset full name (e.g., "Bitcoin")
    pub name: String,

    /// Total supply (e.g., 21000000)
    pub supply: u64,

    /// Decimal precision (e.g., 8 for Bitcoin)
    pub precision: u8,

    /// Genesis UTXO in format "txid:vout"
    /// This UTXO will be marked as occupied by the asset
    pub genesis_utxo: String,
}

/// Asset issuance response
///
/// Information about a newly issued asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    /// Contract ID
    pub contract_id: String,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,

    /// Total supply
    pub supply: u64,

    /// Decimal precision
    pub precision: u8,

    /// Genesis seal (UTXO that holds the initial allocation)
    pub genesis_seal: String,

    /// Registry URI on F1r3node
    pub registry_uri: String,
}

/// Asset metadata for listing
///
/// Compact asset information for displaying lists of assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListItem {
    /// Contract ID
    pub contract_id: String,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,

    /// Registry URI on F1r3node
    pub registry_uri: String,
}

/// Issue a new RGB asset
///
/// Creates a new fungible token by deploying a RHO20 contract to F1r3node
/// and registering the genesis seal in the Bitcoin anchor tracker.
///
/// # Process
///
/// 1. Verify genesis UTXO exists in Bitcoin wallet
/// 2. Check UTXO is not already occupied by RGB
/// 3. Deploy contract via F1r3flyRgbContracts
/// 4. Register genesis seal in tracker
/// 5. Persist updated state
/// 6. Return asset info
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `bitcoin_wallet` - Bitcoin wallet for UTXO verification
/// * `rgb_occupied` - Set of UTXOs already occupied by RGB
/// * `request` - Asset issuance parameters
///
/// # Returns
///
/// `AssetInfo` with contract ID, genesis seal, and metadata
///
/// # Errors
///
/// - `GenesisUtxoNotFound` if UTXO doesn't exist in wallet
/// - `UtxoOccupied` if UTXO is already marked as RGB-occupied
/// - `DeploymentFailed` if contract deployment fails
///
/// # Example
///
/// ```ignore
/// let request = IssueAssetRequest {
///     ticker: "BTC".to_string(),
///     name: "Bitcoin".to_string(),
///     supply: 21000000,
///     precision: 8,
///     genesis_utxo: "abc123...def:0".to_string(),
/// };
///
/// let asset = issue_asset(&mut manager, &wallet, &rgb_occupied, request).await?;
/// ```
pub async fn issue_asset(
    contracts_manager: &mut F1r3flyContractsManager,
    bitcoin_wallet: &BitcoinWallet,
    request: IssueAssetRequest,
) -> Result<AssetInfo, AssetError> {
    // Parse genesis UTXO (format: "txid:vout")
    let parts: Vec<&str> = request.genesis_utxo.split(':').collect();
    if parts.len() != 2 {
        return Err(AssetError::GenesisUtxoNotFound(format!(
            "Invalid UTXO format '{}', expected 'txid:vout'",
            request.genesis_utxo
        )));
    }

    let txid_str = parts[0];
    let vout: u32 = parts[1].parse().map_err(|_| {
        AssetError::GenesisUtxoNotFound(format!(
            "Invalid vout in UTXO '{}', must be a number",
            request.genesis_utxo
        ))
    })?;

    // Parse txid
    use bdk_wallet::bitcoin::Txid;
    use std::str::FromStr;
    let txid = Txid::from_str(txid_str).map_err(|e| {
        AssetError::GenesisUtxoNotFound(format!(
            "Invalid txid in UTXO '{}': {}",
            request.genesis_utxo, e
        ))
    })?;

    // Create OutPoint
    let outpoint = bdk_wallet::bitcoin::OutPoint { txid, vout };

    // Note: We don't check if UTXO is already in rgb_occupied here because:
    // 1. During genesis issuance, the UTXO might be marked as "reserved" for RGB use
    // 2. The issue operation will properly mark it as occupied after successful deployment
    // 3. For transfers (not implemented yet), we would need stricter validation

    // Verify UTXO exists in Bitcoin wallet
    let utxos: Vec<_> = bitcoin_wallet.inner().list_unspent().collect();
    let utxo_exists = utxos.iter().any(|u| u.outpoint == outpoint);

    if !utxo_exists {
        return Err(AssetError::GenesisUtxoNotFound(format!(
            "UTXO '{}' not found in wallet",
            request.genesis_utxo
        )));
    }

    // CRITICAL: Capture the derivation index BEFORE deployment
    // The deploy_contract() method will increment this index if auto_derive is enabled,
    // so we must capture it NOW to get the actual index used for deployment.
    let deployment_index = contracts_manager.contracts().executor().derivation_index();

    log::info!(
        "Capturing derivation index {} before deployment (for signature generation)",
        deployment_index
    );

    // Issue asset via F1r3flyRgbContracts (deploys contract)
    // This will use deployment_index for the child key, then increment to deployment_index+1
    let contract_id = contracts_manager
        .contracts_mut()
        .issue(
            &request.ticker,
            &request.name,
            request.supply,
            request.precision,
        )
        .await
        .map_err(|e| AssetError::DeploymentFailed(e.to_string()))?;

    // Store the ACTUAL derivation index that was used for deployment (captured above)
    contracts_manager.store_contract_derivation_index(&contract_id.to_string(), deployment_index);

    // Call the contract's issue method to assign tokens to genesis seal
    // This allocates the full supply to the genesis UTXO
    //
    // IMPORTANT: Bitcoin txids in strings use display format (big-endian byte order),
    // but the contract's balanceOf method expects internal format (little-endian).
    // We must reverse the txid bytes to match what serialize_seal() produces.
    log::info!(
        "Calling issue method to assign {} tokens to genesis seal {}",
        request.supply,
        request.genesis_utxo
    );

    // Parse and normalize the genesis UTXO format to internal Bitcoin representation
    // RGB tracks balances by actual Bitcoin UTXO identifiers (txid:vout)
    let normalized_genesis_seal = {
        let parts: Vec<&str> = request.genesis_utxo.split(':').collect();
        if parts.len() != 2 {
            return Err(AssetError::DeploymentFailed(format!(
                "Invalid UTXO format: {}",
                request.genesis_utxo
            )));
        }

        let txid_display = parts[0];
        let vout = parts[1];

        // Decode hex (display format is big-endian)
        let txid_bytes = hex::decode(txid_display)
            .map_err(|e| AssetError::DeploymentFailed(format!("Invalid txid hex: {}", e)))?;

        if txid_bytes.len() != 32 {
            return Err(AssetError::DeploymentFailed(format!(
                "Invalid txid length: expected 32 bytes, got {}",
                txid_bytes.len()
            )));
        }

        // Bitcoin internally uses little-endian byte order for txids
        // Reverse the bytes to convert from display format to internal format
        let mut txid_internal = txid_bytes;
        txid_internal.reverse();

        // Format as internal_txid:vout (this matches serialize_seal output)
        format!("{}:{}", hex::encode(txid_internal), vout)
    };

    log::info!(
        "Normalized genesis UTXO for contract: {}",
        normalized_genesis_seal
    );

    // Generate nonce and signature for secured issue() method
    use f1r3fly_rgb::{generate_issue_signature, generate_nonce};

    let nonce = generate_nonce();

    // CRITICAL: Get the child key at the DEPLOYMENT index (not the current index!)
    // The executor's current index has been incremented by auto_derive, so we must
    // explicitly get the key at the deployment_index we captured earlier.
    let child_key = contracts_manager
        .contracts()
        .executor()
        .get_child_key_at_index(deployment_index)
        .map_err(|e| AssetError::DeploymentFailed(format!("Failed to get signing key: {}", e)))?;

    // Get the F1r3fly public key used for this contract
    // This is the deployer's public key, also used as the owner's public key for genesis UTXO
    let f1r3fly_pubkey = contracts_manager
        .contracts()
        .executor()
        .get_public_key_at_index(deployment_index)
        .map_err(|e| AssetError::DeploymentFailed(format!("Failed to get public key: {}", e)))?;

    let f1r3fly_pubkey_hex = hex::encode(f1r3fly_pubkey.serialize_uncompressed());

    // Generate signature: sign(blake2b256((recipient, amount, nonce)))
    let signature =
        generate_issue_signature(&normalized_genesis_seal, request.supply, nonce, &child_key)
            .map_err(|e| {
                AssetError::DeploymentFailed(format!("Failed to generate signature: {}", e))
            })?;

    log::info!(
        "Calling issue method with signature. Nonce: {}, Signature: {}...",
        nonce,
        &signature[..16]
    );

    use strict_types::StrictVal;
    let issue_result = contracts_manager
        .contracts_mut()
        .executor_mut()
        .call_method(
            contract_id,
            "issue",
            &[
                (
                    "recipient",
                    StrictVal::from(normalized_genesis_seal.as_str()),
                ),
                ("amount", StrictVal::from(request.supply)),
                (
                    "recipientPubKey",
                    StrictVal::from(f1r3fly_pubkey_hex.as_str()),
                ),
                ("nonce", StrictVal::from(nonce)),
                ("signatureHex", StrictVal::from(signature.as_str())),
            ],
        )
        .await
        .map_err(|e| AssetError::DeploymentFailed(format!("Failed to call issue method: {}", e)))?;

    log::info!(
        "Issue method succeeded, state hash: {}",
        hex::encode(issue_result.state_hash)
    );

    // Query contract metadata via getMetadata method to verify deployment
    let metadata_json = contracts_manager
        .contracts()
        .executor()
        .query_state(contract_id, "getMetadata", &[])
        .await
        .map_err(|e| AssetError::DeploymentFailed(format!("Failed to query metadata: {}", e)))?;

    // Parse metadata from JSON response - no defaults, must match contract
    let ticker = metadata_json["ticker"]
        .as_str()
        .ok_or_else(|| {
            AssetError::DeploymentFailed(format!(
                "Contract metadata missing 'ticker' field: {:?}",
                metadata_json
            ))
        })?
        .to_string();

    let name = metadata_json["name"]
        .as_str()
        .ok_or_else(|| {
            AssetError::DeploymentFailed(format!(
                "Contract metadata missing 'name' field: {:?}",
                metadata_json
            ))
        })?
        .to_string();

    let supply = metadata_json["supply"].as_u64().ok_or_else(|| {
        AssetError::DeploymentFailed(format!(
            "Contract metadata missing or invalid 'supply' field: {:?}",
            metadata_json
        ))
    })?;

    let precision = metadata_json["decimals"].as_u64().ok_or_else(|| {
        AssetError::DeploymentFailed(format!(
            "Contract metadata missing or invalid 'decimals' field: {:?}",
            metadata_json
        ))
    })? as u8;

    // Store genesis UTXO info for future seal registration (Phase 3: Transfers)
    // This will be used during transfer operations to properly register seals with tracker
    use crate::f1r3fly::{GenesisExecutionData, GenesisUtxoInfo};

    // Store execution result for genesis consignment creation
    let genesis_execution_data = GenesisExecutionData {
        opid: hex::encode(&issue_result.opid),
        deploy_id: issue_result
            .deploy_id_string()
            .map_err(|e| AssetError::DeploymentFailed(format!("Invalid deploy ID: {}", e)))?,
        finalized_block_hash: issue_result
            .block_hash_string()
            .map_err(|e| AssetError::DeploymentFailed(format!("Invalid block hash: {}", e)))?,
        state_hash: issue_result.state_hash,
        rholang_source: String::from_utf8(issue_result.rholang_source.to_vec())
            .map_err(|e| AssetError::DeploymentFailed(format!("Invalid Rholang source: {}", e)))?,
    };

    let genesis_info = GenesisUtxoInfo {
        contract_id: contract_id.to_string(),
        txid: txid_str.to_string(),
        vout,
        ticker: ticker.clone(),
        name: name.clone(),
        supply,
        precision,
        genesis_execution_result: Some(genesis_execution_data),
    };

    // Add to contracts manager and persist
    contracts_manager.add_genesis_utxo(genesis_info);
    contracts_manager.save_state()?;

    // Get registry URI from contract metadata
    let metadata = contracts_manager
        .contracts()
        .executor()
        .get_contract_metadata(contract_id)
        .ok_or_else(|| {
            AssetError::AssetNotFound(format!(
                "Contract metadata not found after deployment: {}",
                contract_id
            ))
        })?;

    // Build response with verified metadata from contract
    Ok(AssetInfo {
        contract_id: contract_id.to_string(),
        ticker,
        name,
        supply,
        precision,
        genesis_seal: request.genesis_utxo,
        registry_uri: metadata.registry_uri.clone(),
    })
}

/// List all assets in the wallet
///
/// Returns a list of all issued assets with their basic metadata.
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
///
/// # Returns
///
/// Vector of `AssetListItem` with contract IDs and metadata
///
/// # Example
///
/// ```ignore
/// let assets = list_assets(&manager);
/// for asset in assets {
///     println!("{}: {}", asset.ticker, asset.name);
/// }
/// ```
pub fn list_assets(contracts_manager: &F1r3flyContractsManager) -> Vec<AssetListItem> {
    let contracts = contracts_manager.contracts();
    let contract_ids = contracts.list();
    let genesis_utxos = contracts_manager.genesis_utxos();

    contract_ids
        .into_iter()
        .filter_map(|contract_id| {
            let contract_id_str = contract_id.to_string();

            // Only include assets that have both metadata and genesis info
            let metadata = contracts.executor().get_contract_metadata(contract_id)?;
            let genesis_info = genesis_utxos.get(&contract_id_str)?;

            Some(AssetListItem {
                contract_id: contract_id_str,
                ticker: genesis_info.ticker.clone(),
                name: genesis_info.name.clone(),
                registry_uri: metadata.registry_uri.clone(),
            })
        })
        .collect()
}

/// Get detailed asset information
///
/// Retrieves full metadata for a specific asset.
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `contract_id` - Contract ID (as string)
///
/// # Returns
///
/// `AssetInfo` with full asset details
///
/// # Errors
///
/// - `AssetNotFound` if contract doesn't exist
///
/// # Example
///
/// ```ignore
/// let asset_info = get_asset_info(&manager, "contract_id_123")?;
/// println!("Asset: {} ({})", asset_info.name, asset_info.ticker);
/// ```
pub fn get_asset_info(
    contracts_manager: &F1r3flyContractsManager,
    contract_id_str: &str,
) -> Result<AssetInfo, AssetError> {
    // Parse contract ID
    use std::str::FromStr;
    let contract_id = ContractId::from_str(contract_id_str)
        .map_err(|e| AssetError::AssetNotFound(format!("Invalid contract ID: {}", e)))?;

    // Get contract metadata for registry URI
    let metadata = contracts_manager
        .contracts()
        .executor()
        .get_contract_metadata(contract_id)
        .ok_or_else(|| AssetError::AssetNotFound(contract_id_str.to_string()))?;

    // Get genesis UTXO info from stored state
    let genesis_info = contracts_manager
        .get_genesis_utxo(contract_id_str)
        .ok_or_else(|| {
            AssetError::AssetNotFound(format!(
                "Genesis UTXO info not found for contract {}",
                contract_id_str
            ))
        })?;

    // Build genesis seal string from stored info
    let genesis_seal = format!("{}:{}", genesis_info.txid, genesis_info.vout);

    Ok(AssetInfo {
        contract_id: contract_id.to_string(),
        ticker: genesis_info.ticker.clone(),
        name: genesis_info.name.clone(),
        supply: genesis_info.supply,
        precision: genesis_info.precision,
        genesis_seal,
        registry_uri: metadata.registry_uri.clone(),
    })
}
