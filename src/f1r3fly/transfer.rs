//! RGB Asset Transfer Operations
//!
//! Implements RGB asset transfers using F1r3fly-RGB protocol:
//! - Parse invoice and validate
//! - Execute F1r3fly contract transfer method
//! - Build Bitcoin witness transaction
//! - Embed Tapret commitment
//! - Create and serialize consignment
//! - Broadcast and persist state

use std::path::PathBuf;

use amplify::confinement::SmallOrdMap;
use bdk_wallet::bitcoin::Amount;
use bp::seals::{Noise, TxoSeal, TxoSealExt, WOutpoint, WTxoSeal};
use bp::{ConsensusDecode, Outpoint, Txid};
use bpstd::psbt::Psbt as BpPsbt;
use std::str::FromStr;
use strict_types::{StrictDumb, StrictVal};

use crate::bitcoin::utxo::FeeRateConfig;
use crate::bitcoin::{BitcoinWallet, EsploraClient};
use crate::f1r3fly::F1r3flyContractsManager;

use bdk_wallet::bitcoin::OutPoint;
use std::collections::HashSet;

/// Error type for transfer operations
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    /// Invoice parsing error
    #[error("Invoice error: {0}")]
    Invoice(#[from] crate::f1r3fly::InvoiceError),

    /// Contract not found
    #[error("Contract not found: {0}")]
    ContractNotFound(String),

    /// Insufficient balance
    #[error("Insufficient balance: need {need}, have {have}")]
    InsufficientBalance { need: u64, have: u64 },

    /// Bitcoin wallet error
    #[error("Bitcoin wallet error: {0}")]
    BitcoinWallet(#[from] crate::bitcoin::BitcoinWalletError),

    /// F1r3fly RGB error
    #[error("F1r3fly RGB error: {0}")]
    F1r3flyRgb(#[from] f1r3fly_rgb::F1r3flyRgbError),

    /// Transaction build error
    #[error("Transaction build failed: {0}")]
    BuildFailed(String),

    /// Transaction sign error
    #[error("Transaction sign failed: {0}")]
    SignFailed(String),

    /// Broadcast error
    #[error("Broadcast failed: {0}")]
    BroadcastFailed(String),

    /// Consignment error
    #[error("Consignment error: {0}")]
    ConsignmentFailed(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Tapret error
    #[error("Tapret error: {0}")]
    Tapret(#[from] f1r3fly_rgb::TapretError),

    /// Invalid seal
    #[error("Invalid seal: {0}")]
    InvalidSeal(String),
}

/// Transfer response with transaction and consignment details
#[derive(Debug, Clone)]
pub struct TransferResponse {
    /// Bitcoin transaction ID (witness transaction)
    pub bitcoin_txid: String,

    /// Consignment filename
    pub consignment_filename: String,

    /// Path to consignment file
    pub consignment_path: PathBuf,

    /// Consignment size in bytes
    pub consignment_size: usize,

    /// Transfer status
    pub status: String,

    /// Amount transferred
    pub amount: u64,

    /// Change amount (kept by sender)
    pub change_amount: u64,
}

/// Send RGB asset transfer
///
/// Complete transfer flow:
/// 1. Parse invoice and validate
/// 2. Execute F1r3fly contract transfer method
/// 3. Build Bitcoin witness transaction
/// 4. Embed Tapret commitment with state hash
/// 5. Sign and broadcast witness transaction
/// 6. Create F1r3flyConsignment
/// 7. Serialize and save consignment to disk
/// 8. Register change seals in tracker
/// 9. Persist state
///
/// # Arguments
///
/// * `bitcoin_wallet` - Bitcoin wallet for witness transaction
/// * `esplora_client` - Esplora client for broadcasting
/// * `contracts_manager` - F1r3fly contracts manager
/// * `invoice_str` - RGB invoice string from recipient
/// * `recipient_pubkey_hex` - Recipient's F1r3fly public key (for transfer authorization)
/// * `fee_rate` - Bitcoin transaction fee rate
/// * `consignments_dir` - Directory to save consignment files
/// * `rgb_occupied` - Set of RGB-occupied UTXOs to protect from spending
///
/// # Returns
///
/// `TransferResponse` with transaction ID and consignment details
///
/// # Errors
///
/// Returns error if:
/// - Invoice parsing fails
/// - Contract not found
/// - Insufficient balance
/// - Transaction build/sign/broadcast fails
/// - Consignment creation fails
pub async fn send_transfer(
    bitcoin_wallet: &mut BitcoinWallet,
    esplora_client: &EsploraClient,
    contracts_manager: &mut F1r3flyContractsManager,
    invoice_str: &str,
    recipient_pubkey_hex: String,
    fee_rate: &FeeRateConfig,
    consignments_dir: PathBuf,
    rgb_occupied: &HashSet<OutPoint>,
) -> Result<TransferResponse, TransferError> {
    log::info!("🚀 Starting RGB transfer");
    log::debug!("  Invoice: {}", invoice_str);

    // ========================================================================
    // Step 1: Parse and Validate Invoice
    // ========================================================================
    log::info!("📄 Step 1: Parsing invoice...");
    let parsed = crate::f1r3fly::parse_invoice(invoice_str)?;
    log::debug!("  Contract ID: {}", parsed.contract_id);
    log::debug!("  Amount: {:?}", parsed.amount);

    let amount = parsed.amount.ok_or_else(|| {
        TransferError::Invoice(crate::f1r3fly::InvoiceError::Core(
            f1r3fly_rgb::F1r3flyRgbError::InvalidResponse(
                "Invoice must specify amount".to_string(),
            ),
        ))
    })?;

    // Get contract ID string and genesis info first (before mutable borrow)
    let contract_id_str = parsed.contract_id.to_string();

    // Get genesis UTXO for seal info
    let genesis_info = contracts_manager
        .get_genesis_utxo(&contract_id_str)
        .ok_or_else(|| {
            TransferError::ContractNotFound(format!(
                "Genesis UTXO not found for contract {}",
                contract_id_str
            ))
        })?
        .clone(); // Clone to avoid lifetime issues

    log::info!(
        "✓ Invoice parsed: {} {} tokens",
        amount,
        genesis_info.ticker
    );

    // ========================================================================
    // Step 2: Check Balance
    // ========================================================================
    log::info!("💰 Step 2: Checking balance...");

    // Get genesis seal as TxoSeal for seal map and balance query
    let genesis_txid = Txid::from_str(&genesis_info.txid)
        .map_err(|e| TransferError::InvalidSeal(format!("Invalid genesis txid: {}", e)))?;
    let genesis_outpoint = Outpoint::new(genesis_txid, genesis_info.vout);
    let genesis_seal = TxoSeal {
        primary: genesis_outpoint,
        secondary: TxoSealExt::Noise(Noise::strict_dumb()),
    };

    // Query balance using actual genesis UTXO identifier
    // RGB tracks balances by Bitcoin UTXO identifiers (txid:vout)
    // Do this BEFORE getting mutable reference to contract
    // CRITICAL: Use serialize_seal() to ensure correct big-endian format
    let genesis_seal_id = f1r3fly_rgb::contract::F1r3flyRgbContract::serialize_seal(&genesis_seal);

    log::debug!("Querying balance for genesis UTXO: {}", genesis_seal_id);

    let balance_result = contracts_manager
        .contracts()
        .executor()
        .query_state(
            parsed.contract_id,
            "balanceOf",
            &[("seal", StrictVal::from(genesis_seal_id.as_str()))],
        )
        .await
        .map_err(|e| TransferError::F1r3flyRgb(e))?;

    let balance = balance_result
        .as_u64()
        .or_else(|| {
            balance_result
                .as_i64()
                .and_then(|n| if n >= 0 { Some(n as u64) } else { None })
        })
        .ok_or_else(|| {
            TransferError::InvalidSeal(format!("Invalid balance response: {:?}", balance_result))
        })?;

    if balance < amount {
        return Err(TransferError::InsufficientBalance {
            need: amount,
            have: balance,
        });
    }

    let change_amount = balance - amount;
    log::info!(
        "✓ Balance sufficient: {} (sending {}+ change {})",
        balance,
        amount,
        change_amount
    );

    // ========================================================================
    // Step 3: Prepare Transfer Parameters
    // ========================================================================
    log::info!("📞 Step 3: Preparing transfer parameters...");

    // Extract recipient seal from parsed invoice
    let recipient_seal = f1r3fly_rgb::extract_seal(&parsed.beneficiary)?;

    // Prepare seals map for transfer
    // Index 0: Genesis seal (input - being spent)
    // Index 1: Recipient seal (output - receiving transferred amount)
    let mut seals_map = SmallOrdMap::new();

    let genesis_wtxo_seal = WTxoSeal {
        primary: WOutpoint::Extern(genesis_seal.primary),
        secondary: genesis_seal.secondary,
    };
    seals_map
        .insert(0u16, genesis_wtxo_seal)
        .map_err(|_| TransferError::InvalidSeal("Failed to insert genesis seal".to_string()))?;

    seals_map
        .insert(1u16, recipient_seal.clone())
        .map_err(|_| TransferError::InvalidSeal("Failed to insert recipient seal".to_string()))?;

    // Serialize seals to UTXO identifiers (txid:vout format) for Rholang
    // RGB tracks balances by actual Bitcoin UTXO identifiers

    // From: Genesis UTXO (already computed above)
    let from_seal_id = genesis_seal_id.clone();

    // To: Recipient UTXO - doesn't exist yet, so use a deterministic placeholder
    // We use a witness identifier based on the recipient address + vout from invoice
    // Returns (seal_id, witness_mapping) where witness_mapping is captured for claim process
    let (to_seal_id, witness_mapping) = match &recipient_seal.primary {
        WOutpoint::Extern(outpoint) => {
            // Recipient UTXO already exists (rare case)
            // CRITICAL: Use serialize_seal() to ensure correct big-endian format
            let recipient_txo_seal = TxoSeal {
                primary: *outpoint,
                secondary: recipient_seal.secondary.clone(),
            };
            let seal_id =
                f1r3fly_rgb::contract::F1r3flyRgbContract::serialize_seal(&recipient_txo_seal);
            (seal_id, None) // No witness mapping needed
        }
        WOutpoint::Wout(vout) => {
            // Witness output: UTXO will be created in this transfer
            // Use recipient address as stable identifier
            let network = bitcoin_wallet.network().to_bitcoin_network();
            let recipient_addr = f1r3fly_rgb::get_recipient_address(&parsed.beneficiary, network)?;

            // Create deterministic identifier: address_hash:vout
            // After broadcast, Bob will query his actual UTXO (the real txid:vout)
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(recipient_addr.as_bytes());
            let addr_hash = hex::encode(&hasher.finalize()[0..16]); // Use first 16 bytes
            let witness_id = format!("witness:{}:{}", addr_hash, vout.into_u32());

            // Create witness mapping for claim process
            let mapping = f1r3fly_rgb::WitnessMapping {
                witness_id: witness_id.clone(),
                recipient_address: recipient_addr,
                expected_vout: vout.into_u32(),
            };

            (witness_id, Some(mapping))
        }
    };

    log::debug!("  From UTXO: {}", from_seal_id);
    log::debug!("  To UTXO: {}", to_seal_id);

    // ========================================================================
    // Generate Transfer Authorization Signature
    // ========================================================================

    // Generate nonce for replay protection
    use f1r3fly_rgb::generate_nonce;
    let transfer_nonce = generate_nonce();

    // Get the contract's derivation index (used when it was deployed)
    let contract_derivation_index = contracts_manager
        .get_contract_derivation_index(&contract_id_str)
        .map_err(|e| {
            TransferError::ContractNotFound(format!(
                "Derivation index not found for contract {}: {}",
                contract_id_str, e
            ))
        })?;

    // Get the child key at that index (this is the owner's signing key)
    let signing_key = contracts_manager
        .contracts()
        .executor()
        .get_child_key_at_index(contract_derivation_index)
        .map_err(|e| TransferError::F1r3flyRgb(e))?;

    // Generate transfer signature: sign(blake2b256((from, to, amount, nonce)))
    use f1r3fly_rgb::generate_transfer_signature;
    let transfer_signature = generate_transfer_signature(
        &from_seal_id,
        &to_seal_id,
        amount,
        transfer_nonce,
        &signing_key,
    )
    .map_err(|e| {
        TransferError::F1r3flyRgb(f1r3fly_rgb::F1r3flyRgbError::InvalidRholangSource(format!(
            "Failed to generate transfer signature: {}",
            e
        )))
    })?;

    log::info!(
        "Generated transfer signature for authorization. Nonce: {}, Signature: {}...",
        transfer_nonce,
        &transfer_signature[..16]
    );

    // ========================================================================
    // Execute F1r3fly Contract Transfer Method
    // ========================================================================

    // Now get mutable contract reference for transfer
    let contract = contracts_manager
        .contracts_mut()
        .get_mut(&parsed.contract_id)
        .ok_or_else(|| TransferError::ContractNotFound(contract_id_str.clone()))?;

    // Call transfer method on contract
    // The Rholang contract will verify signature, deduct from sender, and add to recipient
    let result = contract
        .call_method(
            "transfer",
            &[
                ("from", StrictVal::from(from_seal_id.as_str())),
                ("to", StrictVal::from(to_seal_id.as_str())),
                ("amount", StrictVal::from(amount)),
                ("toPubKey", StrictVal::from(recipient_pubkey_hex.as_str())),
                ("nonce", StrictVal::from(transfer_nonce)),
                (
                    "fromSignatureHex",
                    StrictVal::from(transfer_signature.as_str()),
                ),
            ],
            seals_map.clone(),
        )
        .await?;

    log::info!("✓ F1r3fly transfer executed");
    log::debug!("  State hash: {}", hex::encode(result.state_hash));
    log::debug!(
        "  Block hash: {}",
        result.block_hash_string().unwrap_or_default()
    );

    // ========================================================================
    // Step 4: Build Bitcoin Witness Transaction
    // ========================================================================
    log::info!("⛓️  Step 4: Building Bitcoin witness transaction...");

    // Get recipient address from invoice
    let network = bitcoin_wallet.network().to_bitcoin_network();
    let recipient_addr_str = f1r3fly_rgb::get_recipient_address(&parsed.beneficiary, network)?;

    // DIAGNOSTIC: Log the address extracted from invoice
    log::debug!("🔍 Address from Invoice: {}", recipient_addr_str);

    let recipient_addr = recipient_addr_str
        .parse::<bdk_wallet::bitcoin::Address<bdk_wallet::bitcoin::address::NetworkUnchecked>>()
        .map_err(|e| TransferError::BuildFailed(format!("Invalid recipient address: {}", e)))?
        .assume_checked();

    // Build transaction sending small amount to recipient address
    // This creates the UTXO that will be bound to the RGB tokens
    const DUST_AMOUNT: u64 = 1_000; // 1000 sats minimum

    let mut tx_builder = bitcoin_wallet.inner_mut().build_tx();
    tx_builder.add_recipient(
        recipient_addr.script_pubkey(),
        Amount::from_sat(DUST_AMOUNT),
    );
    tx_builder.fee_rate(fee_rate.to_bdk_fee_rate());

    // CRITICAL SAFETY: Exclude RGB-occupied UTXOs from coin selection
    // This prevents accidental spending of UTXOs that hold RGB assets
    if !rgb_occupied.is_empty() {
        log::debug!(
            "  Protecting {} RGB-occupied UTXO(s) from spending",
            rgb_occupied.len()
        );
        for occupied_outpoint in rgb_occupied {
            tx_builder.add_unspendable(*occupied_outpoint);
        }
    }

    let mut psbt = tx_builder
        .finish()
        .map_err(|e| TransferError::BuildFailed(format!("{}", e)))?;

    log::info!("✓ PSBT built");
    log::debug!("  Outputs: {}", psbt.unsigned_tx.output.len());

    // ========================================================================
    // Step 5: Embed Tapret Commitment in PSBT
    // ========================================================================
    log::info!("🔒 Step 5: Embedding Tapret commitment in PSBT...");

    // Convert BDK PSBT to BP PSBT for Tapret embedding
    let bdk_psbt_bytes = psbt.serialize();
    let mut bp_psbt = BpPsbt::deserialize(&bdk_psbt_bytes).map_err(|e| {
        TransferError::Tapret(f1r3fly_rgb::TapretError::CommitmentFailed(format!(
            "PSBT conversion failed: {:?}",
            e
        )))
    })?;

    // CRITICAL FIX: Ensure tap_internal_key is set for taproot outputs
    // BDK should set this automatically, but we ensure it's present for Tapret embedding
    {
        let mut outputs_vec: Vec<_> = bp_psbt.outputs_mut().collect();
        for (idx, output) in outputs_vec.iter_mut().enumerate() {
            if output.script.is_p2tr() && output.tap_internal_key.is_none() {
                // Extract the internal key from the output script pubkey
                // P2TR script format: OP_1 (0x51) + 32-byte x-only pubkey
                let script_bytes: &[u8] = output.script.as_ref();
                if script_bytes.len() == 34 && script_bytes[0] == 0x51 {
                    // Extract the 32-byte x-only pubkey from the script
                    let xonly_bytes: [u8; 32] = script_bytes[2..34].try_into().map_err(|_| {
                        TransferError::Tapret(f1r3fly_rgb::TapretError::CommitmentFailed(
                            "Failed to extract x-only pubkey from script".to_string(),
                        ))
                    })?;
                    let xonly_pubkey = bp::secp256k1::XOnlyPublicKey::from_slice(&xonly_bytes)
                        .map_err(|e| {
                            TransferError::Tapret(f1r3fly_rgb::TapretError::CommitmentFailed(
                                format!("Invalid x-only pubkey: {}", e),
                            ))
                        })?;
                    output.tap_internal_key = Some(xonly_pubkey.into());
                    log::debug!("✓ Set tap_internal_key for output {}", idx);
                }
            }
        }
    }

    // Embed Tapret commitment in the first output (index 0)
    let tapret_proof = f1r3fly_rgb::embed_tapret_commitment(&mut bp_psbt, 0, result.state_hash)?;

    // Convert BP PSBT back to BDK PSBT
    let bp_psbt_bytes = bp_psbt.serialize(bp_psbt.version);
    psbt = bdk_wallet::bitcoin::Psbt::deserialize(&bp_psbt_bytes)
        .map_err(|e| TransferError::BuildFailed(format!("PSBT conversion back failed: {:?}", e)))?;

    // Create anchor from Tapret proof
    let anchor = f1r3fly_rgb::create_anchor(&tapret_proof)?;

    log::info!("✓ Tapret commitment embedded in PSBT");
    log::debug!("  State hash: {}", hex::encode(result.state_hash));
    log::debug!("  Output index: 0");

    // ========================================================================
    // Step 6: Sign and Broadcast Transaction
    // ========================================================================
    log::info!("✍️  Step 6: Signing and broadcasting...");

    #[allow(deprecated)]
    let sign_options = bdk_wallet::SignOptions::default();
    bitcoin_wallet
        .inner_mut()
        .sign(&mut psbt, sign_options)
        .map_err(|e| TransferError::SignFailed(format!("{}", e)))?;

    let tx = psbt
        .extract_tx()
        .map_err(|e| TransferError::BuildFailed(format!("Extract failed: {}", e)))?;

    let txid = tx.compute_txid();
    log::debug!("  Witness txid: {}", txid);

    // DIAGNOSTIC: Log transaction outputs to verify recipient address
    log::debug!("🔍 Bitcoin TX Outputs ({} outputs):", tx.output.len());
    for (vout, output) in tx.output.iter().enumerate() {
        use bdk_wallet::bitcoin::Address;
        if let Ok(addr) = Address::from_script(&output.script_pubkey, network) {
            log::debug!(
                "  - vout {}: {} ({} sats)",
                vout,
                addr,
                output.value.to_sat()
            );
        } else {
            log::debug!(
                "  - vout {}: <unparseable> ({} sats)",
                vout,
                output.value.to_sat()
            );
        }
    }

    esplora_client
        .inner()
        .broadcast(&tx)
        .map_err(|e| TransferError::BroadcastFailed(format!("{}", e)))?;

    log::info!("✓ Transaction broadcasted: {}", txid);

    // ========================================================================
    // Step 7: Register Anchor in Tracker
    // ========================================================================
    log::info!("📌 Step 7: Registering anchor...");

    // Use the opid from the F1r3fly execution result
    // (state_hash is for Bitcoin commitment, opid is for RGB tracking)
    let opid = result.opid;

    // CRITICAL: Register anchor with the contract's tracker (not contracts_manager tracker)
    // Each contract has its own tracker instance for its operations
    // Get mutable reference to contract and register anchor with its tracker
    let contract = contracts_manager
        .contracts_mut()
        .get_mut(&parsed.contract_id)
        .ok_or_else(|| TransferError::ContractNotFound(contract_id_str.clone()))?;

    contract.tracker_mut().add_anchor(opid, anchor.clone());

    log::info!("✓ Anchor registered");

    // Contract reference will be used for consignment creation below

    // Convert the actual broadcasted witness transaction to bp::Tx for consignment
    // The consignment must contain the real witness transaction with the Tapret commitment
    // We serialize through BDK's format and parse with bpstd's consensus format
    let tx_bytes = bdk_wallet::bitcoin::consensus::encode::serialize(&tx);
    let bc_tx = bpstd::Tx::consensus_deserialize(&tx_bytes[..]).map_err(|e| {
        TransferError::ConsignmentFailed(format!("Failed to deserialize witness TX: {:?}", e))
    })?;
    let bp_tx: bp::Tx = bc_tx.into();

    log::debug!("  Using actual witness TX in consignment: {}", txid);

    // Create consignment with seals and actual witness transaction
    let mut consignment = f1r3fly_rgb::F1r3flyConsignment::new(
        &contract,
        result,
        seals_map,
        vec![bp_tx],
        false, // is_genesis - this is a transfer, not genesis
    )?;

    // Add witness mapping if this is a witness transfer
    consignment.witness_mapping = witness_mapping;

    log::info!("✓ Consignment created");

    // ========================================================================
    // Step 8: Serialize and Save Consignment
    // ========================================================================
    log::info!("💾 Step 8: Saving consignment...");

    let consignment_bytes = consignment.to_bytes()?;
    let consignment_filename = format!("{}.json", txid);

    // Ensure consignments directory exists
    std::fs::create_dir_all(&consignments_dir)?;

    let consignment_path = consignments_dir.join(&consignment_filename);
    std::fs::write(&consignment_path, &consignment_bytes)?;

    log::info!("✓ Consignment saved");
    log::debug!("  Path: {}", consignment_path.display());
    log::debug!("  Size: {} bytes", consignment_bytes.len());

    // ========================================================================
    // Step 9: Persist State
    // ========================================================================
    log::info!("💾 Step 9: Persisting state...");

    bitcoin_wallet.persist()?;
    contracts_manager
        .save_state()
        .map_err(|e| TransferError::ConsignmentFailed(format!("State save failed: {}", e)))?;

    log::info!("✓ State persisted");

    // ========================================================================
    // Success!
    // ========================================================================
    log::info!("✅ Transfer complete!");

    Ok(TransferResponse {
        bitcoin_txid: txid.to_string(),
        consignment_filename,
        consignment_path,
        consignment_size: consignment_bytes.len(),
        status: "broadcasted".to_string(),
        amount,
        change_amount,
    })
}
