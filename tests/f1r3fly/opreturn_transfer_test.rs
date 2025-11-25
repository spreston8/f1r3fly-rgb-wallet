//! OP_RETURN Transfer Integration Tests
//!
//! Tests for RGB asset transfers using OP_RETURN commitments instead of Tapret.
//! This anchoring method is compatible with Lightning Network LDK integration.
//!
//! Tests verify:
//! - OP_RETURN commitment is properly embedded in Bitcoin transactions
//! - State hash can be extracted from OP_RETURN
//! - Full transfer flow works with OP_RETURN (balances, claims, consignments)
//! - Error handling for invalid OP_RETURN extraction
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{
    check_f1r3node_available, issue_test_asset, setup_recipient_wallet, verify_balance_with_retry,
};
use f1r3fly_rgb_wallet::f1r3fly::AnchorMethod;
use std::str::FromStr;

/// Test 1: Complete OP_RETURN transfer flow - Alice sends tokens to Bob
///
/// Verifies:
/// - OP_RETURN commitment is embedded at output index 0
/// - Bitcoin transaction structure is correct (zero value, 34 bytes)
/// - Transfer execution and broadcasting works
/// - Balance updates for sender and recipient
/// - Auto-claim succeeds with OP_RETURN anchored transfer
/// - Total supply conservation
#[tokio::test]
async fn test_opreturn_transfer_alice_to_bob() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("opreturn_transfer");

    println!("\n========================================");
    println!("🧪 Test: OP_RETURN Transfer Alice → Bob");
    println!("========================================");

    // ========================================================================
    // Step 1: Setup - Alice issues 10,000 TEST, Bob accepts genesis
    // ========================================================================
    println!("\n📋 Step 1: Setup wallets and issue asset");
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "OPTEST", 10_000)
            .await
            .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

    // Export and import genesis
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

    println!("✓ Alice issued 10,000 OPTEST");
    println!("✓ Bob imported genesis");

    // ========================================================================
    // Step 2: Bob generates invoice for 2,500 tokens
    // ========================================================================
    println!("\n📋 Step 2: Bob generates invoice");
    let invoice_with_pubkey = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 2_500)
        .expect("Failed to generate invoice");

    let invoice_string = invoice_with_pubkey.invoice_string.clone();
    let recipient_pubkey = invoice_with_pubkey.recipient_pubkey_hex.clone();

    // Bob syncs after revealing invoice address
    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob after invoice generation");

    println!("✓ Invoice generated for 2,500 OPTEST");

    // ========================================================================
    // Step 3: Alice sends transfer with OP_RETURN
    // ========================================================================
    println!("\n📋 Step 3: Alice sends transfer with OP_RETURN anchoring");
    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    let transfer_response = alice
        .send_transfer(
            &invoice_string,
            recipient_pubkey,
            &fee_rate,
            Some(AnchorMethod::OpReturn), // ← OP_RETURN anchoring
        )
        .await
        .expect("Failed to send transfer");

    assert!(
        !transfer_response.bitcoin_txid.is_empty(),
        "Transfer should have bitcoin_txid"
    );
    assert!(
        transfer_response.consignment_path.exists(),
        "Consignment file should exist"
    );
    assert_eq!(
        transfer_response.amount, 2_500,
        "Transfer amount should match invoice"
    );
    assert_eq!(
        transfer_response.change_amount, 7_500,
        "Change amount should be correct"
    );

    println!("✓ Transfer sent: {}", transfer_response.bitcoin_txid);
    println!(
        "  Amount: {} OPTEST, Change: {} OPTEST",
        transfer_response.amount, transfer_response.change_amount
    );

    // ========================================================================
    // Step 4: Wait for Bitcoin confirmation
    // ========================================================================
    println!("\n📋 Step 4: Confirming transaction on-chain");
    env.wait_for_confirmation(&transfer_response.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm transaction");

    println!("✓ Transaction confirmed");

    // ========================================================================
    // Step 5: Verify OP_RETURN commitment in Bitcoin TX
    // ========================================================================
    println!("\n📋 Step 5: Verifying OP_RETURN commitment");
    use bdk_wallet::bitcoin::Txid as BdkTxid;
    let txid_parsed = BdkTxid::from_str(&transfer_response.bitcoin_txid).expect("Invalid txid");

    let tx = env
        .esplora_client
        .inner()
        .get_tx(&txid_parsed)
        .expect("Failed to fetch TX")
        .expect("TX not found");

    // Verify OP_RETURN commitment exists in transaction
    verify_opreturn_in_tx(&tx).expect("OP_RETURN verification failed");

    // Mine confirmation block
    env.mine_blocks(1)
        .expect("Failed to mine confirmation block");

    // Sync wallet to see the confirmed transaction
    alice
        .sync_wallet()
        .await
        .expect("Failed to sync Alice wallet");

    // ========================================================================
    // Step 6: Verify Alice's balance
    // ========================================================================
    println!("\n📋 Step 6: Verifying Alice's balance");
    verify_balance_with_retry(&mut alice, &asset_info.contract_id, 7_500, 20)
        .await
        .expect("Alice balance mismatch");

    println!("✓ Alice balance: 7,500 OPTEST");

    // ========================================================================
    // Step 7: Bob syncs and accepts consignment
    // ========================================================================
    println!("\n📋 Step 7: Bob accepts consignment");
    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob before accepting");

    bob.accept_consignment(
        transfer_response
            .consignment_path
            .to_str()
            .expect("Invalid consignment path"),
    )
    .await
    .expect("Failed to accept consignment");

    println!("✓ Consignment accepted");

    // ========================================================================
    // Step 8: Verify Bob's balance (auto-claim should have succeeded)
    // ========================================================================
    println!("\n📋 Step 8: Verifying Bob's balance");
    verify_balance_with_retry(&mut bob, &asset_info.contract_id, 2_500, 20)
        .await
        .expect("Bob balance mismatch");

    println!("✓ Bob balance: 2,500 OPTEST (auto-claim succeeded)");

    // ========================================================================
    // Step 9: Verify total supply conservation
    // ========================================================================
    println!("\n📋 Step 9: Verifying supply conservation");
    let alice_balance = alice
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to get Alice balance");

    let bob_balance = bob
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to get Bob balance");

    assert_eq!(
        alice_balance.total + bob_balance.total,
        10_000,
        "Total supply should be conserved: Alice {} + Bob {} != 10,000",
        alice_balance.total,
        bob_balance.total
    );

    println!("✓ Total supply conserved: 10,000 OPTEST");
    println!(
        "  Alice: {} + Bob: {} = 10,000",
        alice_balance.total, bob_balance.total
    );

    println!("\n✅ OP_RETURN transfer test PASSED");
}

/// Test 2: OP_RETURN commitment extraction and verification
///
/// Verifies:
/// - State hash can be extracted from OP_RETURN output
/// - Extracted hash is 32 bytes
/// - Consignment validation succeeds (implicit verification of correct hash)
#[tokio::test]
async fn test_opreturn_commitment_extraction() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("opreturn_extraction");

    println!("\n========================================");
    println!("🧪 Test: OP_RETURN Commitment Extraction");
    println!("========================================");

    // ========================================================================
    // Setup and transfer (abbreviated)
    // ========================================================================
    println!("\n📋 Setup: Creating transfer with OP_RETURN");
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "EXTRACT", 5_000)
            .await
            .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

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

    let invoice_with_pubkey = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 1_000)
        .expect("Failed to generate invoice");

    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob after invoice generation");

    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    let transfer_response = alice
        .send_transfer(
            &invoice_with_pubkey.invoice_string,
            invoice_with_pubkey.recipient_pubkey_hex,
            &fee_rate,
            Some(AnchorMethod::OpReturn),
        )
        .await
        .expect("Failed to send transfer");

    println!("✓ Transfer created: {}", transfer_response.bitcoin_txid);

    // ========================================================================
    // Wait for confirmation
    // ========================================================================
    env.wait_for_confirmation(&transfer_response.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // ========================================================================
    // Extract and verify OP_RETURN commitment
    // ========================================================================
    println!("\n📋 Step: Extracting state hash from OP_RETURN");
    use bdk_wallet::bitcoin::Txid as BdkTxid;
    let txid_parsed = BdkTxid::from_str(&transfer_response.bitcoin_txid).expect("Invalid txid");

    let bdk_tx = env
        .esplora_client
        .inner()
        .get_tx(&txid_parsed)
        .expect("Failed to fetch TX")
        .expect("TX not found");

    // Convert BDK transaction to bp::Tx for OP_RETURN extraction
    use bp::ConsensusDecode;
    let tx_bytes = bdk_wallet::bitcoin::consensus::encode::serialize(&bdk_tx);
    let bc_tx =
        bpstd::Tx::consensus_deserialize(&tx_bytes[..]).expect("Failed to deserialize witness TX");
    let bp_tx: bp::Tx = bc_tx.into();

    // Extract commitment using f1r3fly-rgb's extraction function
    let extracted_hash = f1r3fly_rgb::extract_opreturn_commitment(&bp_tx, 0)
        .expect("Failed to extract OP_RETURN commitment");

    println!("✓ State hash extracted from OP_RETURN:");
    println!("  Hash: {}", hex::encode(extracted_hash));
    println!("  Length: 32 bytes");

    // Verify it's a valid 32-byte hash
    assert_eq!(
        extracted_hash.len(),
        32,
        "Extracted hash should be 32 bytes"
    );

    // ========================================================================
    // Verify consignment validates successfully
    // ========================================================================
    println!("\n📋 Step: Verifying consignment accepts (validates commitment)");
    env.mine_blocks(1)
        .expect("Failed to mine confirmation block");

    alice.sync_wallet().await.expect("Failed to sync Alice");
    bob.sync_wallet().await.expect("Failed to sync Bob");

    // If consignment acceptance succeeds, it means the OP_RETURN hash was correct
    bob.accept_consignment(
        transfer_response
            .consignment_path
            .to_str()
            .expect("Invalid consignment path"),
    )
    .await
    .expect("Failed to accept consignment - OP_RETURN commitment may be invalid");

    println!("✓ Consignment accepted (OP_RETURN commitment validated)");

    // Verify balance to confirm transfer worked
    verify_balance_with_retry(&mut bob, &asset_info.contract_id, 1_000, 20)
        .await
        .expect("Bob balance mismatch");

    println!("✓ Bob balance correct: 1,000 EXTRACT");

    println!("\n✅ OP_RETURN extraction test PASSED");
}

/// Test 3: Error handling for invalid OP_RETURN extraction
///
/// Verifies:
/// - Extracting from non-OP_RETURN output returns error
/// - Extracting from invalid output index returns error
/// - Error messages are meaningful
#[tokio::test]
async fn test_opreturn_extraction_error_handling() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("opreturn_errors");

    println!("\n========================================");
    println!("🧪 Test: OP_RETURN Error Handling");
    println!("========================================");

    // ========================================================================
    // Create a normal Tapret transfer (NO OP_RETURN)
    // ========================================================================
    println!("\n📋 Setup: Creating Tapret transfer (no OP_RETURN)");
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "ERRTEST", 3_000)
            .await
            .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

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

    let invoice_with_pubkey = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 500)
        .expect("Failed to generate invoice");

    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob after invoice generation");

    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    // Send with Tapret (default, no OP_RETURN)
    let transfer_response = alice
        .send_transfer(
            &invoice_with_pubkey.invoice_string,
            invoice_with_pubkey.recipient_pubkey_hex,
            &fee_rate,
            None, // ← Tapret, NOT OP_RETURN
        )
        .await
        .expect("Failed to send transfer");

    println!(
        "✓ Tapret transfer created: {}",
        transfer_response.bitcoin_txid
    );

    // Wait for confirmation
    env.wait_for_confirmation(&transfer_response.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // Fetch transaction
    use bdk_wallet::bitcoin::Txid as BdkTxid;
    let txid_parsed = BdkTxid::from_str(&transfer_response.bitcoin_txid).expect("Invalid txid");

    let bdk_tx = env
        .esplora_client
        .inner()
        .get_tx(&txid_parsed)
        .expect("Failed to fetch TX")
        .expect("TX not found");

    // Convert BDK transaction to bp::Tx for OP_RETURN extraction
    use bp::ConsensusDecode;
    let tx_bytes = bdk_wallet::bitcoin::consensus::encode::serialize(&bdk_tx);
    let bc_tx =
        bpstd::Tx::consensus_deserialize(&tx_bytes[..]).expect("Failed to deserialize witness TX");
    let bp_tx: bp::Tx = bc_tx.into();

    // ========================================================================
    // Test 3a: Extract from non-OP_RETURN output should fail
    // ========================================================================
    println!("\n📋 Test 3a: Extracting from non-OP_RETURN output");
    let result = f1r3fly_rgb::extract_opreturn_commitment(&bp_tx, 0);

    assert!(
        result.is_err(),
        "Should fail when extracting from non-OP_RETURN output"
    );

    if let Err(e) = result {
        println!("✓ Got expected error: {}", e);
        // Verify error is NotOpReturn variant
        assert!(
            e.to_string().contains("not an OP_RETURN"),
            "Error should indicate output is not OP_RETURN"
        );
    }

    // ========================================================================
    // Test 3b: Extract from invalid output index should fail
    // ========================================================================
    println!("\n📋 Test 3b: Extracting from invalid output index");
    let result = f1r3fly_rgb::extract_opreturn_commitment(&bp_tx, 999);

    assert!(
        result.is_err(),
        "Should fail when output index is out of bounds"
    );

    if let Err(e) = result {
        println!("✓ Got expected error: {}", e);
        // Verify error is InvalidOutputIndex variant
        assert!(
            e.to_string().contains("Invalid output index")
                || e.to_string().contains("out of bounds"),
            "Error should indicate invalid output index"
        );
    }

    // ========================================================================
    // Test 3c: Verify transaction has NO OP_RETURN (sanity check)
    // ========================================================================
    println!("\n📋 Test 3c: Verifying transaction has no OP_RETURN");
    let has_opreturn = bdk_tx
        .output
        .iter()
        .any(|output| output.script_pubkey.is_op_return());

    assert!(
        !has_opreturn,
        "Tapret transaction should not have OP_RETURN outputs"
    );

    println!("✓ Confirmed: No OP_RETURN outputs in Tapret transaction");

    println!("\n✅ Error handling test PASSED");
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Verify OP_RETURN commitment exists in Bitcoin transaction
///
/// Checks:
/// 1. Transaction has OP_RETURN output at index 0
/// 2. OP_RETURN contains exactly 32-byte state hash
/// 3. OP_RETURN value is zero
/// 4. Script format is correct (34 bytes total)
fn verify_opreturn_in_tx(
    tx: &bdk_wallet::bitcoin::Transaction,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check output 0 exists
    if tx.output.is_empty() {
        return Err("Transaction has no outputs".into());
    }

    let output = &tx.output[0];

    // Check is OP_RETURN
    if !output.script_pubkey.is_op_return() {
        return Err("Output 0 is not OP_RETURN".into());
    }

    // Check value is zero
    if output.value != bdk_wallet::bitcoin::Amount::ZERO {
        return Err("OP_RETURN output should have zero value".into());
    }

    // Check OP_RETURN data length
    // Format: OP_RETURN (1) + PUSHBYTES_32 (1) + 32 bytes = 34 bytes total
    let script_bytes = output.script_pubkey.as_bytes();
    if script_bytes.len() != 34 {
        return Err(format!(
            "OP_RETURN script should be 34 bytes, got {}",
            script_bytes.len()
        )
        .into());
    }

    println!("✓ OP_RETURN commitment verified:");
    println!("  - Output index: 0");
    println!("  - Value: 0 sats");
    println!("  - Script length: 34 bytes");
    println!("  - State hash: {}", hex::encode(&script_bytes[2..34]));

    Ok(())
}
