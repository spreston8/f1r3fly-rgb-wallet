//! RGB Balance Queries
//!
//! Handles querying RGB token balances by mapping Bitcoin UTXOs to RGB seals
//! and querying contract state on F1r3node.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use bdk_wallet::bitcoin::OutPoint as BdkOutPoint;
use f1r3fly_rgb::TxoSeal;

use crate::bitcoin::BitcoinWallet;
use crate::f1r3fly::contracts::F1r3flyContractsManager;

/// Error type for balance operations
#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    /// Contract not found
    #[error("Contract not found: {0}")]
    ContractNotFound(String),

    /// Balance query failed
    #[error("Balance query failed: {0}")]
    QueryFailed(String),

    /// F1r3fly-RGB error
    #[error("F1r3fly-RGB error: {0}")]
    F1r3flyRgb(#[from] f1r3fly_rgb::F1r3flyRgbError),

    /// Invalid UTXO format
    #[error("Invalid UTXO format: {0}")]
    InvalidUtxo(String),
}

/// RGB balance for a single asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetBalance {
    /// Contract ID
    pub contract_id: String,

    /// Asset ticker
    pub ticker: String,

    /// Asset name
    pub name: String,

    /// Total balance (sum of all UTXOs holding this asset)
    pub total: u64,

    /// Decimal precision (0 = indivisible, 8 = like BTC)
    pub precision: u8,

    /// Per-UTXO balances
    pub utxo_balances: Vec<UtxoBalance>,
}

/// Balance for a specific UTXO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoBalance {
    /// UTXO outpoint (txid:vout)
    pub outpoint: String,

    /// Token amount held by this UTXO
    pub amount: u64,
}

/// UTXO with RGB asset information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbOccupiedUtxo {
    /// UTXO outpoint (txid:vout)
    pub outpoint: String,

    /// Contract ID (if known)
    pub contract_id: Option<String>,

    /// Asset ticker (if known)
    pub ticker: Option<String>,

    /// Token amount (if known)
    pub amount: Option<u64>,
}

/// Get RGB balance for all assets
///
/// Queries F1r3node contract state for each asset and maps Bitcoin wallet UTXOs
/// to RGB seals to calculate per-asset and per-UTXO balances.
///
/// # Process
///
/// 1. Get all issued assets from contracts manager
/// 2. Get all UTXOs from Bitcoin wallet
/// 3. For each asset:
///    - For each UTXO:
///      - Convert UTXO to TxoSeal
///      - Query contract.balance(seal) on F1r3node
///      - Aggregate balances
/// 4. Return per-contract balance breakdown
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `bitcoin_wallet` - Bitcoin wallet (for UTXO enumeration)
///
/// # Returns
///
/// Vector of `AssetBalance` (one per asset with non-zero balance)
///
/// # Errors
///
/// Returns error if contract queries fail or UTXO conversion fails
///
/// # Example
///
/// ```ignore
/// let balances = get_rgb_balance(&mut contracts_manager, &bitcoin_wallet).await?;
/// for balance in balances {
///     println!("{} ({}): {}", balance.name, balance.ticker, balance.total);
/// }
/// ```
pub async fn get_rgb_balance(
    contracts_manager: &mut F1r3flyContractsManager,
    bitcoin_wallet: &BitcoinWallet,
) -> Result<Vec<AssetBalance>, BalanceError> {
    let mut asset_balances = Vec::new();

    // Get all contracts from manager
    let contract_ids = contracts_manager.contracts().list();

    log::info!(
        "📊 get_rgb_balance: Querying {} contracts",
        contract_ids.len()
    );

    // Query balance for each contract using get_asset_balance()
    // This ensures we use the full logic including claimed UTXOs fallback
    for contract_id in contract_ids {
        let contract_id_str = contract_id.to_string();

        log::info!("  Querying contract: {}", contract_id_str);

        match get_asset_balance(contracts_manager, bitcoin_wallet, &contract_id_str).await {
            Ok(balance) => {
                log::info!("    Total balance: {}", balance.total);
                asset_balances.push(balance);
            }
            Err(e) => {
                log::warn!(
                    "Failed to query balance for contract {}: {}",
                    contract_id_str,
                    e
                );
                // Continue with other contracts even if one fails
            }
        }
    }

    Ok(asset_balances)
}

/// Get RGB balance for a specific asset
///
/// Queries F1r3node contract state for a single asset across all wallet UTXOs.
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `bitcoin_wallet` - Bitcoin wallet
/// * `contract_id` - Contract ID as string
///
/// # Returns
///
/// `AssetBalance` for the specified contract
///
/// # Errors
///
/// Returns error if contract not found or query fails
///
/// # Example
///
/// ```ignore
/// let balance = get_asset_balance(&mut contracts_manager, &bitcoin_wallet, "contract_id").await?;
/// println!("Total: {}", balance.total);
/// ```
pub async fn get_asset_balance(
    contracts_manager: &mut F1r3flyContractsManager,
    bitcoin_wallet: &BitcoinWallet,
    contract_id_str: &str,
) -> Result<AssetBalance, BalanceError> {
    // Parse contract ID
    use std::str::FromStr;
    let contract_id = f1r3fly_rgb::ContractId::from_str(contract_id_str)
        .map_err(|e| BalanceError::ContractNotFound(format!("Invalid contract ID: {}", e)))?;

    // Get asset metadata (clone early to avoid borrow conflicts)
    let (ticker, name, precision) = {
        let genesis_info = contracts_manager
            .get_genesis_utxo(contract_id_str)
            .ok_or_else(|| {
                BalanceError::ContractNotFound(format!(
                    "Genesis info not found for contract {}",
                    contract_id_str
                ))
            })?;
        (
            genesis_info.ticker.clone(),
            genesis_info.name.clone(),
            genesis_info.precision,
        )
    };

    // Get claimed UTXOs early (before borrowing contracts_mut)
    let claimed_utxos = contracts_manager
        .claim_storage()
        .get_claimed_utxos(contract_id_str)
        .unwrap_or_default();

    // Get contract instance
    let contract = contracts_manager
        .contracts_mut()
        .get(&contract_id)
        .ok_or_else(|| BalanceError::ContractNotFound(contract_id_str.to_string()))?;

    // Get all wallet UTXOs for balance queries
    // RGB tracks balances by Bitcoin UTXO identifiers
    let bdk_utxos: Vec<_> = bitcoin_wallet.inner().list_unspent().collect();

    log::info!("================================================");
    log::info!("📊 BALANCE QUERY for contract {}", contract_id_str);
    log::info!("================================================");
    log::info!("  BDK UTXOs:     {} to query", bdk_utxos.len());
    log::info!("  Claimed UTXOs: {} to query", claimed_utxos.len());
    log::info!("------------------------------------------------");

    if !bdk_utxos.is_empty() {
        log::info!("  BDK UTXOs (from list_unspent):");
        for utxo in &bdk_utxos {
            log::info!("    • {}:{}", utxo.outpoint.txid, utxo.outpoint.vout);
        }
    } else {
        log::info!("  ⚠️  No BDK UTXOs found via list_unspent()");
    }

    if !claimed_utxos.is_empty() {
        log::info!("  Claimed UTXOs (from claim storage):");
        for (txid, vout) in &claimed_utxos {
            log::info!("    • {}:{}", txid, vout);
        }
    } else {
        log::info!("  ⚠️  No claimed UTXOs found in claim storage");
    }
    log::info!("================================================");

    let mut utxo_balances = Vec::new();
    let mut total_balance = 0u64;

    // Query balance for each BDK-tracked UTXO
    for utxo in &bdk_utxos {
        let seal = convert_outpoint_to_seal(&utxo.outpoint)?;

        log::debug!(
            "Querying BDK UTXO {}:{} for contract {}",
            utxo.outpoint.txid,
            utxo.outpoint.vout,
            contract_id_str
        );

        match contract.balance(&seal).await {
            Ok(amount) if amount > 0 => {
                log::info!(
                    "  ✅ BDK UTXO {}:{} has balance: {} (found via BDK list_unspent)",
                    utxo.outpoint.txid,
                    utxo.outpoint.vout,
                    amount
                );
                utxo_balances.push(UtxoBalance {
                    outpoint: format!("{}:{}", utxo.outpoint.txid, utxo.outpoint.vout),
                    amount,
                });
                total_balance += amount;
            }
            Ok(amount) => {
                log::debug!(
                    "  - UTXO {}:{} has zero balance ({})",
                    utxo.outpoint.txid,
                    utxo.outpoint.vout,
                    amount
                );
            }
            Err(e) => {
                log::debug!(
                    "  ✗ Balance query failed for UTXO {}:{}: {}",
                    utxo.outpoint.txid,
                    utxo.outpoint.vout,
                    e
                );
            }
        }
    }

    // PRODUCTION FIX: Also query RGB-specific claimed UTXOs
    // These UTXOs may have Tapret-modified addresses that BDK doesn't track,
    // so we maintain separate tracking in our claim storage.
    log::debug!(
        "Querying {} claimed RGB UTXOs for contract {}",
        claimed_utxos.len(),
        contract_id_str
    );

    for (txid_str, vout) in claimed_utxos {
        // Skip if already queried via BDK
        let already_queried = bdk_utxos
            .iter()
            .any(|u| u.outpoint.txid.to_string() == txid_str && u.outpoint.vout == vout);

        if already_queried {
            log::debug!(
                "  Skipping claimed UTXO {}:{} (already queried via BDK)",
                txid_str,
                vout
            );
            continue;
        }

        // Parse txid and create outpoint
        use bdk_wallet::bitcoin::Txid;
        use std::str::FromStr;
        let txid = match Txid::from_str(&txid_str) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Invalid txid in claimed UTXO: {}: {}", txid_str, e);
                continue;
            }
        };

        let outpoint = bdk_wallet::bitcoin::OutPoint::new(txid, vout);
        let seal = convert_outpoint_to_seal(&outpoint)?;

        // DIAGNOSTIC: Show the exact seal_id that will be sent to F1r3node
        let seal_id_for_f1r3node = f1r3fly_rgb::F1r3flyRgbContract::serialize_seal(&seal);

        log::info!(
            "🔍 Querying claimed RGB UTXO {}:{} for contract {}",
            txid,
            vout,
            contract_id_str
        );
        log::info!("   Seal ID sent to F1r3node: {}", seal_id_for_f1r3node);

        match contract.balance(&seal).await {
            Ok(amount) if amount > 0 => {
                log::info!(
                    "  ✅ CLAIMED UTXO {}:{} has balance: {} (found via claim storage fallback)",
                    txid,
                    vout,
                    amount
                );
                utxo_balances.push(UtxoBalance {
                    outpoint: format!("{}:{}", txid, vout),
                    amount,
                });
                total_balance += amount;
            }
            Ok(amount) => {
                log::debug!(
                    "  - RGB UTXO {}:{} has zero balance ({}) (from claim tracking)",
                    txid,
                    vout,
                    amount
                );
            }
            Err(e) => {
                log::debug!(
                    "  ✗ Balance query failed for RGB UTXO {}:{}: {}",
                    txid,
                    vout,
                    e
                );
            }
        }
    }

    Ok(AssetBalance {
        contract_id: contract_id_str.to_string(),
        ticker,
        name,
        total: total_balance,
        precision,
        utxo_balances,
    })
}

/// Get list of UTXOs occupied by RGB assets
///
/// Returns all wallet UTXOs that hold RGB tokens, with asset information if available.
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `bitcoin_wallet` - Bitcoin wallet
///
/// # Returns
///
/// Vector of `RgbOccupiedUtxo` with UTXO details and asset info
///
/// # Example
///
/// ```ignore
/// let occupied = get_occupied_utxos(&mut contracts_manager, &bitcoin_wallet).await?;
/// for utxo in occupied {
///     println!("UTXO {} holds {} tokens", utxo.outpoint, utxo.amount.unwrap_or(0));
/// }
/// ```
pub async fn get_occupied_utxos(
    contracts_manager: &mut F1r3flyContractsManager,
    bitcoin_wallet: &BitcoinWallet,
) -> Result<Vec<RgbOccupiedUtxo>, BalanceError> {
    let mut occupied_utxos = Vec::new();

    // Build a map of outpoint -> (contract_id, ticker, amount)
    let mut utxo_map: HashMap<String, (String, String, u64)> = HashMap::new();

    // Get all contracts
    let contract_ids = contracts_manager.contracts().list();
    let bdk_utxos: Vec<_> = bitcoin_wallet.inner().list_unspent().collect();

    // Get all claimed RGB UTXOs across all contracts (from claim storage)
    // These UTXOs may have Tapret-modified addresses that BDK doesn't track
    let mut all_claimed_utxos: Vec<(String, u32, String)> = Vec::new(); // (txid, vout, contract_id)

    for contract_id in &contract_ids {
        let contract_id_str = contract_id.to_string();
        if let Ok(claimed) = contracts_manager
            .claim_storage()
            .get_claimed_utxos(&contract_id_str)
        {
            for (txid, vout) in claimed {
                all_claimed_utxos.push((txid, vout, contract_id_str.clone()));
            }
        }
    }

    // Query each contract for each UTXO
    for contract_id in contract_ids {
        let contract_id_str = contract_id.to_string();

        // Get asset metadata (clone ticker to avoid borrow conflicts)
        let ticker = match contracts_manager.get_genesis_utxo(&contract_id_str) {
            Some(info) => info.ticker.clone(),
            None => continue,
        };

        // Get contract instance
        let contract = match contracts_manager.contracts_mut().get(&contract_id) {
            Some(c) => c,
            None => continue,
        };

        // Query balance for each BDK-tracked UTXO
        for utxo in &bdk_utxos {
            let outpoint_str = format!("{}:{}", utxo.outpoint.txid, utxo.outpoint.vout);
            let seal = convert_outpoint_to_seal(&utxo.outpoint)?;

            if let Ok(amount) = contract.balance(&seal).await {
                if amount > 0 {
                    // Store or aggregate
                    utxo_map.insert(
                        outpoint_str,
                        (contract_id_str.clone(), ticker.clone(), amount),
                    );
                }
            }
        }

        // PRODUCTION FIX: Also query claimed RGB UTXOs
        // These may have Tapret-modified addresses not tracked by BDK
        for (txid_str, vout, claimed_contract_id) in &all_claimed_utxos {
            // Skip if this contract doesn't match
            if claimed_contract_id != &contract_id_str {
                continue;
            }

            // Skip if already queried via BDK
            let already_queried = bdk_utxos
                .iter()
                .any(|u| u.outpoint.txid.to_string() == *txid_str && u.outpoint.vout == *vout);
            if already_queried {
                continue;
            }

            // Parse txid and create outpoint
            use bdk_wallet::bitcoin::Txid;
            use std::str::FromStr;
            let txid = match Txid::from_str(txid_str) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("Invalid txid in claimed UTXO: {}: {}", txid_str, e);
                    continue;
                }
            };

            let outpoint = bdk_wallet::bitcoin::OutPoint::new(txid, *vout);
            let outpoint_str = format!("{}:{}", txid, vout);
            let seal = convert_outpoint_to_seal(&outpoint)?;

            if let Ok(amount) = contract.balance(&seal).await {
                if amount > 0 {
                    log::debug!(
                        "RGB UTXO {}:{} occupied by {} (from claim tracking)",
                        txid,
                        vout,
                        ticker
                    );
                    utxo_map.insert(
                        outpoint_str,
                        (contract_id_str.clone(), ticker.clone(), amount),
                    );
                }
            }
        }
    }

    // Convert map to vec
    for (outpoint, (contract_id, ticker, amount)) in utxo_map {
        occupied_utxos.push(RgbOccupiedUtxo {
            outpoint,
            contract_id: Some(contract_id),
            ticker: Some(ticker),
            amount: Some(amount),
        });
    }

    Ok(occupied_utxos)
}

/// Get RGB seal information for a specific UTXO
///
/// Queries F1r3node contracts to determine which RGB assets (if any) are bound
/// to the specified UTXO. This enriches Bitcoin UTXO data with RGB metadata.
///
/// # Process
///
/// 1. Parse outpoint string to BDK OutPoint
/// 2. Convert to TxoSeal for RGB queries
/// 3. Query all contracts for balance at this seal
/// 4. Build `RgbSealInfo` for each contract with non-zero balance
///
/// # Arguments
///
/// * `contracts_manager` - F1r3fly contracts manager
/// * `outpoint_str` - UTXO outpoint as "txid:vout" string
///
/// # Returns
///
/// Vector of `crate::types::RgbSealInfo` (one per asset on this UTXO).
/// Returns empty vector if UTXO holds no RGB assets.
///
/// # Errors
///
/// Returns error if:
/// - Outpoint format is invalid
/// - Contract queries fail critically
///
/// # Example
///
/// ```ignore
/// use f1r3fly_rgb_wallet::f1r3fly::balance::get_rgb_seal_info;
///
/// let rgb_assets = get_rgb_seal_info(
///     &mut contracts_manager,
///     "abc123...def:0"
/// ).await?;
///
/// for asset in rgb_assets {
///     println!("{}: {} tokens", asset.ticker, asset.amount.unwrap_or(0));
/// }
/// ```
pub async fn get_rgb_seal_info(
    contracts_manager: &mut F1r3flyContractsManager,
    outpoint_str: &str,
) -> Result<Vec<crate::types::RgbSealInfo>, BalanceError> {
    use crate::types::RgbSealInfo;

    // Parse outpoint string "txid:vout"
    let parts: Vec<&str> = outpoint_str.split(':').collect();
    if parts.len() != 2 {
        return Err(BalanceError::InvalidUtxo(format!(
            "Invalid outpoint format '{}', expected 'txid:vout'",
            outpoint_str
        )));
    }

    // Parse txid and vout
    use std::str::FromStr;
    let txid = bdk_wallet::bitcoin::Txid::from_str(parts[0])
        .map_err(|e| BalanceError::InvalidUtxo(format!("Invalid txid '{}': {}", parts[0], e)))?;
    let vout: u32 = parts[1]
        .parse()
        .map_err(|e| BalanceError::InvalidUtxo(format!("Invalid vout '{}': {}", parts[1], e)))?;

    // Create BDK OutPoint
    let outpoint = BdkOutPoint { txid, vout };

    // Convert to RGB TxoSeal
    let seal = convert_outpoint_to_seal(&outpoint)?;

    let mut rgb_seals = Vec::new();

    // Query all contracts for balances at this seal
    let contract_ids = contracts_manager.contracts().list();

    for contract_id in contract_ids {
        let contract_id_str = contract_id.to_string();

        // Get asset metadata (clone to avoid borrow conflicts)
        let ticker = match contracts_manager.get_genesis_utxo(&contract_id_str) {
            Some(info) => info.ticker.clone(),
            None => continue, // Skip if no genesis info
        };

        // Get contract instance
        let contract = match contracts_manager.contracts_mut().get(&contract_id) {
            Some(c) => c,
            None => continue, // Skip if contract not found
        };

        // Query balance at this seal
        match contract.balance(&seal).await {
            Ok(amount) if amount > 0 => {
                // This UTXO holds tokens for this contract
                rgb_seals.push(RgbSealInfo {
                    contract_id: contract_id_str,
                    ticker,
                    amount: Some(amount),
                });
            }
            Ok(_) => {
                // Zero balance, UTXO doesn't hold this asset
            }
            Err(e) => {
                // Query failed, log but continue
                // (UTXO might not be relevant to this contract)
                log::debug!(
                    "Balance query failed for UTXO {} on contract {}: {}",
                    outpoint_str,
                    contract_id_str,
                    e
                );
            }
        }
    }

    Ok(rgb_seals)
}

/// Convert BDK OutPoint to RGB TxoSeal
///
/// Converts a Bitcoin UTXO reference to an RGB seal for balance queries.
///
/// # Arguments
///
/// * `outpoint` - BDK OutPoint reference
///
/// # Returns
///
/// `TxoSeal` for RGB balance queries
///
/// # Errors
///
/// Returns error if conversion fails
fn convert_outpoint_to_seal(outpoint: &BdkOutPoint) -> Result<TxoSeal, BalanceError> {
    use f1r3fly_rgb::Txid as RgbTxid;

    // Convert bitcoin::Txid to RGB Txid (bp::Txid)
    // Both store txid bytes in the same internal format (little-endian)
    // We can directly copy the bytes without any conversion
    let txid_bytes: [u8; 32] = *outpoint.txid.as_ref();
    let rgb_txid = RgbTxid::from(txid_bytes);

    // Create outpoint
    let rgb_outpoint = bp::Outpoint::new(rgb_txid, outpoint.vout);

    // Create TxoSeal with primary outpoint and no fallback (secondary = Noise)
    use bp::seals::{Noise, TxoSealExt};
    use strict_types::StrictDumb;

    Ok(TxoSeal {
        primary: rgb_outpoint,
        secondary: TxoSealExt::Noise(Noise::strict_dumb()),
    })
}
