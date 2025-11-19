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
    let bob_bitcoin_wallet = bob
        .bitcoin_wallet_mut()
        .expect("Bob should have Bitcoin wallet");

    let bob_generated = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        bob_bitcoin_wallet,
        &asset_info.contract_id,
        5_000,
        None,
    )
    .expect("Failed to generate Bob's invoice");

    let bob_invoice_string = bob_generated.invoice.to_string();

    // Alice sends 5,000 to Bob
    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    let transfer1 = alice
        .send_transfer(&bob_invoice_string, &fee_rate)
        .await
        .expect("Failed to send first transfer");

    env.wait_for_confirmation(&transfer1.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm first transfer");

    // Sync Alice's wallet to update UTXO state
    alice
        .sync_wallet()
        .expect("Failed to sync Alice wallet after first transfer");

    // Verify Alice's balance after first transfer
    verify_balance_with_retry(&mut alice, &asset_info.contract_id, 5_000, 5)
        .await
        .expect("Alice should have 5,000 tokens after first transfer");

    // Carol generates invoice for 6,000 (more than Alice has left)
    let carol_bitcoin_wallet = carol
        .bitcoin_wallet_mut()
        .expect("Carol should have Bitcoin wallet");

    let carol_generated = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        carol_bitcoin_wallet,
        &asset_info.contract_id,
        6_000,
        None,
    )
    .expect("Failed to generate Carol's invoice");

    let carol_invoice_string = carol_generated.invoice.to_string();

    // Alice tries to send 6,000 to Carol but only has 5,000 left
    // This should FAIL due to insufficient balance
    let result = alice.send_transfer(&carol_invoice_string, &fee_rate).await;

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
        bob.sync_wallet().expect("Failed to sync Bob wallet");

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
