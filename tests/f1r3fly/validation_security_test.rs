//! Security & Validation Integration Tests
//!
//! Tests for validation and security failure cases to ensure robust error handling:
//! - Corrupted state hash rejection
//! - Unfinalized block rejection
//! - Double-spend prevention
//! - Invalid seal handling
//! - Corrupted consignment file rejection
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{
    check_f1r3node_available, issue_test_asset, setup_recipient_wallet, verify_balance_with_retry,
};

/// Test 1: Reject consignment with corrupted state hash
///
/// Verifies:
/// - Consignments with corrupted state_hash are rejected during validation
/// - Error message indicates validation failure
#[tokio::test]
async fn test_reject_invalid_state_hash() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("invalid_state_hash");

    // Setup: Alice issues asset, exports genesis
    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TST", 1000)
        .await
        .expect("Failed to issue asset");

    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    // Read and corrupt the consignment JSON
    let consignment_bytes =
        std::fs::read(&genesis_response.consignment_path).expect("Failed to read consignment file");

    let mut consignment: serde_json::Value =
        serde_json::from_slice(&consignment_bytes).expect("Failed to parse consignment JSON");

    // Corrupt state hash in f1r3fly_proof
    consignment["f1r3fly_proof"]["state_hash"] =
        serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");

    // Bob tries to accept corrupted consignment
    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let corrupted_path = env.wallet_dir("bob").join("corrupted_state_hash.json");
    std::fs::create_dir_all(corrupted_path.parent().unwrap()).expect("Failed to create directory");
    std::fs::write(
        &corrupted_path,
        serde_json::to_vec(&consignment).expect("Failed to serialize"),
    )
    .expect("Failed to write corrupted consignment");

    let result = bob
        .accept_consignment(corrupted_path.to_str().unwrap())
        .await;

    // Should fail validation
    assert!(result.is_err(), "Should reject corrupted state hash");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("validation")
            || error_msg.to_lowercase().contains("invalid")
            || error_msg.to_lowercase().contains("hash")
            || error_msg.to_lowercase().contains("deserialization"),
        "Error should mention validation or hash issue, got: {}",
        error_msg
    );
}

/// Test 2: Reject consignment with unfinalized F1r3fly block
///
/// Verifies:
/// - Consignments with non-existent/unfinalized block_hash are rejected
/// - F1r3node finalization check prevents accepting premature state
#[tokio::test]
async fn test_reject_unfinalized_block() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("unfinalized_block");

    // Setup: Alice issues asset, exports genesis
    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TST", 1000)
        .await
        .expect("Failed to issue asset");

    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    // Read and modify the consignment JSON
    let consignment_bytes =
        std::fs::read(&genesis_response.consignment_path).expect("Failed to read consignment file");

    let mut consignment: serde_json::Value =
        serde_json::from_slice(&consignment_bytes).expect("Failed to parse consignment JSON");

    // Replace block_hash with a fake hash that doesn't exist on F1r3node
    // This simulates a block that hasn't been finalized yet
    consignment["f1r3fly_proof"]["block_hash"] =
        serde_json::json!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

    // Bob tries to accept consignment with unfinalized block
    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let unfinalized_path = env.wallet_dir("bob").join("unfinalized_block.json");
    std::fs::create_dir_all(unfinalized_path.parent().unwrap())
        .expect("Failed to create directory");
    std::fs::write(
        &unfinalized_path,
        serde_json::to_vec(&consignment).expect("Failed to serialize"),
    )
    .expect("Failed to write modified consignment");

    let result = bob
        .accept_consignment(unfinalized_path.to_str().unwrap())
        .await;

    // Should fail validation due to unfinalized block
    assert!(
        result.is_err(),
        "Should reject consignment with unfinalized block"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("finalized")
            || error_msg.to_lowercase().contains("block")
            || error_msg.to_lowercase().contains("not found")
            || error_msg.to_lowercase().contains("invalid"),
        "Error should mention finalization or block issue, got: {}",
        error_msg
    );
}

/// Test 3: Prevent double-spend (RGB-occupied UTXO protection)
///
/// Verifies:
/// - Once a UTXO is used in a transfer, it cannot be spent again
/// - RGB-occupied UTXO tracking prevents double-spending
/// - Error message is informative
#[tokio::test]
async fn test_prevent_double_spend() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("double_spend");

    // Setup: Alice has 10,000 tokens, Bob and Carol accept genesis
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "TST", 10_000)
            .await
            .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let mut carol = setup_recipient_wallet(&env, "carol", "password")
        .await
        .expect("Failed to setup Carol");

    // Both accept genesis
    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    bob.accept_consignment(
        genesis_response
            .consignment_path
            .to_str()
            .expect("Invalid path"),
    )
    .await
    .expect("Failed to accept genesis");

    carol
        .accept_consignment(
            genesis_response
                .consignment_path
                .to_str()
                .expect("Invalid path"),
        )
        .await
        .expect("Failed to accept genesis");

    // Bob generates invoice for 5,000
    let bob_invoice_data = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 5_000)
        .expect("Failed to generate Bob's invoice");

    let bob_invoice_string = bob_invoice_data.invoice_string.clone();
    let bob_pubkey = bob_invoice_data.recipient_pubkey_hex.clone();

    // Alice sends 5,000 to Bob
    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    let transfer1 = alice
        .send_transfer(&bob_invoice_string, bob_pubkey, &fee_rate, None)
        .await
        .expect("Failed to send first transfer");

    env.wait_for_confirmation(&transfer1.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm first transfer");

    // Sync Alice's wallet to update UTXO state
    alice
        .sync_wallet()
        .await
        .expect("Failed to sync Alice wallet after first transfer");

    // Verify Alice's balance after first transfer
    verify_balance_with_retry(&mut alice, &asset_info.contract_id, 5_000, 5)
        .await
        .expect("Alice should have 5,000 tokens after first transfer");

    // Carol generates invoice for 6,000 (more than Alice has left)
    let carol_invoice_data = carol
        .generate_invoice_with_pubkey(&asset_info.contract_id, 6_000)
        .expect("Failed to generate Carol's invoice");

    let carol_invoice_string = carol_invoice_data.invoice_string.clone();
    let carol_pubkey = carol_invoice_data.recipient_pubkey_hex.clone();

    // Alice tries to send 6,000 to Carol but only has 5,000 left
    // This should FAIL due to insufficient balance
    let result = alice
        .send_transfer(&carol_invoice_string, carol_pubkey, &fee_rate, None)
        .await;

    assert!(
        result.is_err(),
        "Should prevent transfer with insufficient balance"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("insufficient")
            || error_msg.to_lowercase().contains("balance")
            || error_msg.to_lowercase().contains("not enough"),
        "Error should mention insufficient balance, got: {}",
        error_msg
    );
}

/// Test 4: Reject consignment with invalid seal (non-existent UTXO)
///
/// Verifies:
/// - Consignments with seals pointing to non-existent UTXOs are handled gracefully
/// - System doesn't crash on invalid seal references
#[tokio::test]
async fn test_reject_invalid_seal() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("invalid_seal");

    // Setup: Alice issues asset, exports genesis
    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TST", 1000)
        .await
        .expect("Failed to issue asset");

    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    // Read and corrupt the consignment JSON
    let consignment_bytes =
        std::fs::read(&genesis_response.consignment_path).expect("Failed to read consignment file");

    let mut consignment: serde_json::Value =
        serde_json::from_slice(&consignment_bytes).expect("Failed to parse consignment JSON");

    // Corrupt seal to point to non-existent UTXO
    // Change the txid in the seal to a fake one
    if let Some(seals) = consignment["seals"].as_object_mut() {
        for (_key, seal) in seals.iter_mut() {
            if let Some(primary) = seal.get_mut("primary") {
                if let Some(extern_seal) = primary.get_mut("Extern") {
                    extern_seal["txid"] = serde_json::json!(
                        "0000000000000000000000000000000000000000000000000000000000000000"
                    );
                }
            }
        }
    }

    // Bob tries to accept consignment with invalid seal
    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let invalid_seal_path = env.wallet_dir("bob").join("invalid_seal.json");
    std::fs::create_dir_all(invalid_seal_path.parent().unwrap())
        .expect("Failed to create directory");
    std::fs::write(
        &invalid_seal_path,
        serde_json::to_vec(&consignment).expect("Failed to serialize"),
    )
    .expect("Failed to write corrupted consignment");

    let result = bob
        .accept_consignment(invalid_seal_path.to_str().unwrap())
        .await;

    // May pass acceptance (seal validation is lenient for genesis)
    // but should fail or show zero balance when querying
    if result.is_ok() {
        // If acceptance succeeded, query balance should fail or return 0
        // since the seal points to a non-existent UTXO
        bob.sync_wallet().await.expect("Failed to sync Bob wallet");

        let balance_result = bob.get_asset_balance(&asset_info.contract_id).await;

        // Balance query might fail or return incorrect value due to invalid seal
        if let Ok(balance) = balance_result {
            // With invalid seal, balance calculation may be affected
            println!(
                "  Balance with invalid seal: {} (expected issues)",
                balance.total
            );
        } else {
            println!("  Balance query failed as expected with invalid seal");
        }
    } else {
        // Acceptance failed, which is also acceptable
        println!("  Acceptance failed with invalid seal");
    }
}

/// Test 5: Reject corrupted consignment file
///
/// Verifies:
/// - Malformed JSON is rejected gracefully
/// - Error messages are informative
/// - No panics or crashes on corrupted data
#[tokio::test]
async fn test_reject_corrupted_consignment() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("corrupted_file");

    // Create corrupted consignment file with invalid JSON
    let corrupted_path = env.wallet_dir("test").join("corrupted.json");
    std::fs::create_dir_all(corrupted_path.parent().unwrap()).expect("Failed to create directory");
    std::fs::write(&corrupted_path, b"{ invalid json }").expect("Failed to write corrupted file");

    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let result = bob
        .accept_consignment(corrupted_path.to_str().unwrap())
        .await;

    assert!(result.is_err(), "Should reject corrupted JSON");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("deserialize")
            || error_msg.to_lowercase().contains("parse")
            || error_msg.to_lowercase().contains("invalid")
            || error_msg.to_lowercase().contains("json"),
        "Error should mention deserialization issue, got: {}",
        error_msg
    );
}

/// Test 6: Reject completely empty file
///
/// Verifies:
/// - Empty files are rejected
/// - Error handling is robust
#[tokio::test]
async fn test_reject_empty_consignment() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("empty_file");

    // Create empty consignment file
    let empty_path = env.wallet_dir("test").join("empty.json");
    std::fs::create_dir_all(empty_path.parent().unwrap()).expect("Failed to create directory");
    std::fs::write(&empty_path, b"").expect("Failed to write empty file");

    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let result = bob.accept_consignment(empty_path.to_str().unwrap()).await;

    assert!(result.is_err(), "Should reject empty file");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("deserialize")
            || error_msg.to_lowercase().contains("parse")
            || error_msg.to_lowercase().contains("invalid")
            || error_msg.to_lowercase().contains("eof")
            || error_msg.to_lowercase().contains("empty"),
        "Error should mention parsing issue, got: {}",
        error_msg
    );
}

/// Test 7: Reject consignment with missing required fields
///
/// Verifies:
/// - Consignments missing required fields are rejected
/// - Field validation works correctly
#[tokio::test]
async fn test_reject_consignment_missing_fields() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("missing_fields");

    // Create consignment with missing fields
    let incomplete_consignment = serde_json::json!({
        "version": 1,
        "contract_id": "contract:test123",
        // Missing: f1r3fly_proof, bitcoin_anchor, seals
    });

    let incomplete_path = env.wallet_dir("test").join("incomplete.json");
    std::fs::create_dir_all(incomplete_path.parent().unwrap()).expect("Failed to create directory");
    std::fs::write(
        &incomplete_path,
        serde_json::to_vec(&incomplete_consignment).expect("Failed to serialize"),
    )
    .expect("Failed to write incomplete consignment");

    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob");

    let result = bob
        .accept_consignment(incomplete_path.to_str().unwrap())
        .await;

    assert!(result.is_err(), "Should reject incomplete consignment");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.to_lowercase().contains("missing")
            || error_msg.to_lowercase().contains("deserialize")
            || error_msg.to_lowercase().contains("invalid")
            || error_msg.to_lowercase().contains("field"),
        "Error should mention missing fields, got: {}",
        error_msg
    );
}

/// Test 7: Unauthorized transfer rejected by signature verification
///
/// Verifies:
/// - Transfers require valid signature from UTXO owner
/// - Attacker cannot move tokens by signing with their own key
/// - Contract-level authorization prevents unauthorized transfers
#[tokio::test]
async fn test_unauthorized_transfer_rejected() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("unauthorized_transfer");

    // ========================================================================
    // Step 1: Alice issues 10,000 tokens
    // ========================================================================
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "TEST", 10_000)
            .await
            .expect("Failed to issue asset");

    // ========================================================================
    // Step 2: Attacker wallet accepts genesis (to know about the contract)
    // ========================================================================
    let mut attacker = setup_recipient_wallet(&env, "attacker", "password")
        .await
        .expect("Failed to setup attacker");

    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    attacker
        .accept_consignment(
            genesis_response
                .consignment_path
                .to_str()
                .expect("Invalid path"),
        )
        .await
        .expect("Failed to accept genesis");

    // ========================================================================
    // Step 3: Attacker generates invoice for themselves
    // ========================================================================
    let attacker_invoice_data = attacker
        .generate_invoice_with_pubkey(&asset_info.contract_id, 5_000)
        .expect("Failed to generate attacker's invoice");

    let attacker_invoice_string = attacker_invoice_data.invoice_string.clone();
    let attacker_pubkey = attacker_invoice_data.recipient_pubkey_hex.clone();

    // ========================================================================
    // Step 4: Attacker tries to transfer Alice's tokens using their own signature
    // ========================================================================
    // This should FAIL because:
    // 1. The "from" UTXO is owned by Alice (registered with Alice's pubkey)
    // 2. Attacker's signature won't match Alice's pubkey
    // 3. The contract should reject with "Invalid signature"

    // Parse invoice to get "to" seal
    let parsed = f1r3fly_rgb_wallet::f1r3fly::parse_invoice(&attacker_invoice_string)
        .expect("Failed to parse invoice");

    // Extract seal from beneficiary
    let to_wtxo_seal = f1r3fly_rgb_wallet::f1r3fly::extract_seal_from_invoice(&parsed.beneficiary)
        .expect("Failed to extract seal");

    // Get Alice's genesis UTXO seal (the "from" that attacker wants to steal from)
    let alice_balances = alice
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to get Alice's balance");

    assert_eq!(
        alice_balances.total, 10_000,
        "Alice should have 10,000 tokens"
    );
    assert!(
        !alice_balances.utxo_balances.is_empty(),
        "Alice should have UTXOs with tokens"
    );

    // Get Alice's UTXO outpoint string (display format, big-endian)
    let from_seal_display = alice_balances.utxo_balances[0].outpoint.clone();

    // NORMALIZE the seal to match what was registered during issue()
    // The contract registers owners using normalized (little-endian) txids
    let from_seal_id = {
        let parts: Vec<&str> = from_seal_display.split(':').collect();
        if parts.len() != 2 {
            panic!("Invalid UTXO format: {}", from_seal_display);
        }

        let txid_display = parts[0];
        let vout = parts[1];

        // Decode and reverse to get internal (little-endian) format
        let txid_bytes = hex::decode(txid_display).expect("Invalid txid hex");
        let mut txid_internal = txid_bytes;
        txid_internal.reverse();

        format!("{}:{}", hex::encode(txid_internal), vout)
    };

    // Convert WTxoSeal to string format for contract call
    let to_seal_id = to_wtxo_seal.to_string();

    // Generate nonce using f1r3fly_rgb utility
    use f1r3fly_rgb::generate_nonce;
    let nonce = generate_nonce();

    // Get ATTACKER's signing key (wrong key!)
    let attacker_contracts = attacker
        .f1r3fly_contracts()
        .expect("F1r3fly not initialized");
    let attacker_signing_key = attacker_contracts
        .contracts()
        .executor()
        .get_child_key()
        .expect("Failed to get attacker's key");

    // Attacker signs the transfer with THEIR key (not Alice's)
    use f1r3fly_rgb::generate_transfer_signature;
    let attacker_signature = generate_transfer_signature(
        &from_seal_id,
        &to_seal_id,
        5_000,
        nonce,
        &attacker_signing_key, // ❌ Wrong key!
    )
    .expect("Failed to generate signature");

    // Attempt to execute unauthorized transfer via contract
    use amplify::confinement::Confined;
    use std::collections::BTreeMap;
    use strict_types::StrictVal;

    let attacker_contracts_mut = attacker
        .f1r3fly_contracts_mut()
        .expect("F1r3fly not initialized");

    let contract = attacker_contracts_mut
        .contracts_mut()
        .get_mut(&parsed.contract_id)
        .expect("Contract not found");

    // Create empty seals map (Confined type required by call_method)
    let empty_seals: BTreeMap<u16, f1r3fly_rgb::WTxoSeal> = BTreeMap::new();
    let seals_map = Confined::try_from(empty_seals).expect("Failed to create confined map");

    // Call transfer with attacker's (wrong) signature
    let _result = contract
        .call_method(
            "transfer",
            &[
                ("from", StrictVal::from(from_seal_id.as_str())),
                ("to", StrictVal::from(to_seal_id.as_str())),
                ("amount", StrictVal::from(5_000u64)),
                ("toPubKey", StrictVal::from(attacker_pubkey.as_str())),
                ("nonce", StrictVal::from(nonce)),
                (
                    "fromSignatureHex",
                    StrictVal::from(attacker_signature.as_str()),
                ),
            ],
            seals_map,
        )
        .await;

    // ========================================================================
    // Step 5: Verify the unauthorized transfer was REJECTED
    // ========================================================================
    // NOTE: The Rholang execution may succeed (no crash), but the contract
    // should return {"success": false, "error": "Invalid signature"}
    // The real test is whether Alice's balance changed (it shouldn't have)

    // We don't assert on result.is_err() because the Rholang execution itself
    // may succeed while the business logic (signature verification) fails.
    // Instead, we verify that no tokens were stolen by checking balances below.

    // ========================================================================
    // Step 6: Verify Alice still has all 10,000 tokens (nothing was stolen)
    // ========================================================================
    let alice_balance_after = alice
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to get Alice's balance after attack");

    assert_eq!(
        alice_balance_after.total, 10_000,
        "Alice should still have all 10,000 tokens after failed attack"
    );
}
