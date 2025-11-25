//! Complete Transfer Flow Integration Tests
//!
//! Tests for full RGB asset transfer lifecycle including:
//! - Invoice generation and parsing
//! - Transfer execution with Bitcoin anchoring
//! - Tapret commitment verification
//! - Balance updates and state consistency
//! - RGB-occupied UTXO tracking
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{
    check_f1r3node_available, issue_test_asset, setup_recipient_wallet, verify_balance_with_retry,
};
use std::str::FromStr;

/// Test complete transfer flow: Alice sends tokens to Bob
///
/// Verifies:
/// - Invoice generation and parsing
/// - Transfer execution and Bitcoin anchoring
/// - Tapret commitment on-chain
/// - Balance updates for sender and recipient
/// - RGB-occupied UTXO tracking
/// - Total supply conservation
#[tokio::test]
async fn test_complete_transfer_alice_to_bob() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("complete_transfer");

    // ========================================================================
    // Step 1: Setup - Alice issues 10,000 TEST, Bob accepts genesis
    // ========================================================================
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "TEST", 10_000)
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

    // ========================================================================
    // Step 2: Bob generates invoice for 2,500 tokens
    // ========================================================================
    let invoice_with_pubkey = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 2_500)
        .expect("Failed to generate invoice");

    let invoice_string = invoice_with_pubkey.invoice_string.clone();
    let recipient_pubkey = invoice_with_pubkey.recipient_pubkey_hex.clone();

    // ========================================================================
    // Step 2.5: Bob syncs after revealing invoice address
    // ========================================================================
    // CRITICAL: After reveal_next_address() in invoice generation, Bob must sync
    // so that BDK's in-memory spk_index tracks the new address for UTXO discovery.
    // Without this sync, BDK won't discover UTXOs sent to the invoice address!
    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob after invoice generation");

    // ========================================================================
    // Step 3: Alice sends transfer
    // ========================================================================
    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();

    let transfer_response = alice
        .send_transfer(&invoice_string, recipient_pubkey, &fee_rate, None)
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

    // ========================================================================
    // Step 4: Wait for Bitcoin confirmation
    // ========================================================================
    env.wait_for_confirmation(&transfer_response.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // ========================================================================
    // Step 5: Verify Tapret commitment in Bitcoin TX
    // ========================================================================
    use bdk_wallet::bitcoin::Txid as BdkTxid;
    let txid_parsed = BdkTxid::from_str(&transfer_response.bitcoin_txid).expect("Invalid txid");

    let tx = env
        .esplora_client
        .inner()
        .get_tx(&txid_parsed)
        .expect("Failed to fetch TX")
        .expect("TX not found");

    // Verify Tapret commitment exists in transaction
    verify_tapret_in_tx(&tx).expect("Tapret verification failed");

    // ========================================================================
    // Step 5.5: Confirm the transfer transaction on-chain
    // ========================================================================
    // Mine a block to confirm the transfer transaction
    env.mine_blocks(1)
        .expect("Failed to mine confirmation block");

    // Sync wallet to see the confirmed transaction
    alice
        .sync_wallet()
        .await
        .expect("Failed to sync Alice wallet");

    // ========================================================================
    // Step 6: Verify Alice's balance (with retry for F1r3fly state delays)
    // ========================================================================
    verify_balance_with_retry(&mut alice, &asset_info.contract_id, 7_500, 20)
        .await
        .expect("Alice balance mismatch");

    // ========================================================================
    // Step 7: Verify Alice's RGB-occupied UTXOs tracked
    // ========================================================================
    let alice_occupied = alice
        .get_occupied_utxos()
        .await
        .expect("Failed to get occupied UTXOs");

    assert!(
        !alice_occupied.is_empty(),
        "Alice should have RGB-occupied UTXOs (change seal)"
    );

    // ========================================================================
    // Step 7.5: Bob syncs wallet to detect received UTXO
    // ========================================================================
    // Bob needs to sync his wallet BEFORE accepting the consignment
    // so his BDK wallet can discover the new UTXO that Alice sent him.
    // This UTXO will be used during the auto-claim process when accepting the consignment.

    bob.sync_wallet().await.expect("Failed to sync Bob wallet");

    // ========================================================================
    // Step 8: Bob accepts consignment
    // ========================================================================
    bob.accept_consignment(
        transfer_response
            .consignment_path
            .to_str()
            .expect("Invalid path"),
    )
    .await
    .expect("Failed to accept consignment");

    // ========================================================================
    // Step 8.5: Bob syncs wallet again to trigger auto-claim
    // ========================================================================
    // After accepting the consignment (which stores the witness mapping),
    // Bob needs to sync again. This triggers retry_pending_claims which will
    // find the stored witness mapping and execute the claim to migrate the
    // balance from the witness ID to Bob's real UTXO.
    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob wallet after accept");

    // ========================================================================
    // Step 9: Verify Bob's balance
    // ========================================================================
    verify_balance_with_retry(&mut bob, &asset_info.contract_id, 2_500, 20)
        .await
        .expect("Bob balance mismatch");

    // ========================================================================
    // Step 9.5: Verify claim was recorded in database (Production Fix Validation)
    // ========================================================================
    // With the production fix, the actual UTXO should be extracted from the
    // consignment witness transaction and stored in the claim record.
    let claim_storage = bob
        .f1r3fly_contracts()
        .expect("Bob should have contracts manager")
        .claim_storage();

    let claims = claim_storage
        .get_all_claims(&asset_info.contract_id)
        .expect("Failed to query claims");

    assert_eq!(claims.len(), 1, "Should have exactly 1 claim record");

    let claim = &claims[0];
    assert_eq!(
        claim.status,
        f1r3fly_rgb_wallet::storage::ClaimStatus::Claimed,
        "Claim should have status Claimed (auto-claim succeeded)"
    );
    assert!(
        claim.claimed_at.is_some(),
        "Claim should have claimed_at timestamp"
    );

    // Production fix validation: actual UTXO should be extracted from consignment
    assert!(
        claim.actual_txid.is_some(),
        "Claim should have actual_txid from consignment witness transaction"
    );
    assert!(
        claim.actual_vout.is_some(),
        "Claim should have actual_vout from consignment witness transaction"
    );

    println!(
        "✓ Claim verified: {} (status: {:?}, actual UTXO: {}:{})",
        claim.witness_id,
        claim.status,
        claim.actual_txid.as_ref().unwrap(),
        claim.actual_vout.unwrap()
    );

    // ========================================================================
    // Step 10: Verify total supply conservation
    // ========================================================================
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
}

/// Helper to verify Tapret commitment exists in Bitcoin transaction
///
/// Verifies that the transaction contains a taproot output suitable for
/// Tapret commitments. This ensures the RGB state is properly anchored
/// to the Bitcoin blockchain.
///
/// Full cryptographic verification is performed by F1r3flyConsignment::validate()
/// during acceptance, so this helper performs a basic sanity check that the
/// transaction structure is compatible with Tapret (has taproot outputs).
fn verify_tapret_in_tx(
    tx: &bdk_wallet::bitcoin::Transaction,
) -> Result<(), Box<dyn std::error::Error>> {
    // Tapret commitments are embedded in taproot outputs
    // We need to check if any output contains our commitment

    // For now, we verify that:
    // 1. Transaction has outputs (basic sanity check)
    // 2. At least one output is a taproot output (witness v1)

    if tx.output.is_empty() {
        return Err("Transaction has no outputs".into());
    }

    let mut has_taproot = false;
    for output in &tx.output {
        // Check if output is witness v1 (taproot)
        // Taproot scripts start with OP_1 (0x51) followed by 32 bytes
        let script_bytes = output.script_pubkey.as_bytes();
        if script_bytes.len() == 34 && script_bytes[0] == 0x51 {
            has_taproot = true;
            break;
        }
    }

    if !has_taproot {
        return Err("Transaction has no taproot outputs (Tapret requires taproot)".into());
    }

    // Note: Full cryptographic verification of the Tapret proof would require:
    // 1. Extracting the taproot script tree from the witness data
    // 2. Locating the Tapret leaf in the tree
    // 3. Verifying the commitment hash matches expected_state_hash
    //
    // This is already done by F1r3flyConsignment::validate() during acceptance,
    // so this helper just performs a basic sanity check that the TX structure
    // is compatible with Tapret (has taproot outputs).

    Ok(())
}
