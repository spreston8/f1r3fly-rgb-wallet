//! Claim Flow Integration Tests
//!
//! Tests for witness claim operations including:
//! - Auto-claim on consignment acceptance
//! - Claim retry mechanism on wallet sync
//! - Claim storage tracking (pending → claimed transitions)
//! - Actual UTXO extraction from consignment witness transactions
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{
    check_f1r3node_available, issue_test_asset, setup_recipient_wallet, verify_balance_with_retry,
};

/// Test claim works even when accepting consignment before confirmation
///
/// With the production fix (actual UTXO from consignment), claims succeed
/// immediately even if the Bitcoin TX isn't confirmed yet, because we use
/// the actual txid:vout from the consignment witness transaction rather than
/// relying on BDK address discovery.
///
/// This test validates:
/// 1. Transfer is broadcast but NOT yet confirmed
/// 2. Bob accepts consignment → auto-claim SUCCEEDS (using actual UTXO from consignment)
/// 3. Claim is stored as "Claimed" in database
/// 4. Balance is correct even before confirmation
///
/// This demonstrates that recipients can accept consignments optimistically
/// before Bitcoin confirmation and claims work immediately.
#[tokio::test]
async fn test_claim_before_confirmation() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("claim_retry");

    // ========================================================================
    // Step 1: Setup - Alice issues asset, Bob accepts genesis
    // ========================================================================
    println!("\n📋 Step 1: Setting up Alice and Bob");

    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "TEST", 10_000)
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

    println!("✓ Alice issued 10,000 TEST tokens");
    println!("✓ Bob accepted genesis");

    // ========================================================================
    // Step 2: Bob generates invoice
    // ========================================================================
    println!("\n📋 Step 2: Bob generates invoice for 2,500 tokens");

    let invoice_with_pubkey = bob
        .generate_invoice_with_pubkey(&asset_info.contract_id, 2_500)
        .expect("Failed to generate invoice");

    bob.sync_wallet()
        .await
        .expect("Failed to sync Bob after invoice");

    println!("✓ Invoice generated");

    // ========================================================================
    // Step 3: Alice sends transfer (NOT yet confirmed)
    // ========================================================================
    println!("\n📋 Step 3: Alice sends transfer (broadcast but not confirmed)");

    let fee_rate = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
    let transfer_response = alice
        .send_transfer(
            &invoice_with_pubkey.invoice_string,
            invoice_with_pubkey.recipient_pubkey_hex,
            &fee_rate,
        )
        .await
        .expect("Failed to send transfer");

    println!("✓ Transfer broadcast: {}", transfer_response.bitcoin_txid);
    println!("⏳ NOT mining blocks yet - TX unconfirmed");

    // ========================================================================
    // Step 4: Bob accepts consignment BEFORE confirmation
    // ========================================================================
    println!("\n📋 Step 4: Bob accepts consignment (before Bitcoin confirmation)");

    bob.accept_consignment(
        transfer_response
            .consignment_path
            .to_str()
            .expect("Invalid path"),
    )
    .await
    .expect("Failed to accept consignment");

    println!("✓ Consignment accepted");
    println!(
        "  (With production fix, auto-claim should SUCCEED using actual UTXO from consignment)"
    );

    // ========================================================================
    // Step 5: Verify claim is CLAIMED (succeeded immediately)
    // ========================================================================
    println!("\n📋 Step 5: Verify claim status is CLAIMED (immediate success)");

    let claim_storage = bob
        .f1r3fly_contracts()
        .expect("Bob should have contracts manager")
        .claim_storage();

    let claims = claim_storage
        .get_all_claims(&asset_info.contract_id)
        .expect("Failed to query claims");

    assert_eq!(claims.len(), 1, "Should have 1 claim record");

    let claim = &claims[0];
    assert_eq!(
        claim.status,
        f1r3fly_rgb_wallet::storage::ClaimStatus::Claimed,
        "Claim should be Claimed (auto-claim succeeded using actual UTXO from consignment)"
    );
    assert!(
        claim.claimed_at.is_some(),
        "Claim should have claimed_at timestamp"
    );

    println!("✓ Claim status verified: {:?}", claim.status);
    println!("  Witness ID: {}", claim.witness_id);
    println!(
        "  Claimed at: {} (before Bitcoin confirmation!)",
        claim.claimed_at.unwrap_or_default()
    );

    // ========================================================================
    // Step 6: Verify Bob's balance BEFORE confirmation
    // ========================================================================
    println!("\n📋 Step 6: Verify Bob's balance (even before confirmation)");

    verify_balance_with_retry(&mut bob, &asset_info.contract_id, 2_500, 20)
        .await
        .expect("Bob balance mismatch");

    println!("✓ Bob has correct balance: 2,500 tokens (before Bitcoin confirmation!)");

    // ========================================================================
    // Step 7: Verify actual UTXO was extracted from consignment (production fix)
    // ========================================================================
    println!("\n📋 Step 7: Verify actual UTXO from consignment (production fix)");

    assert!(
        claim.actual_txid.is_some(),
        "Claim should have actual_txid from consignment"
    );
    assert!(
        claim.actual_vout.is_some(),
        "Claim should have actual_vout from consignment"
    );

    println!(
        "✓ Actual UTXO verified: {}:{}",
        claim.actual_txid.as_ref().unwrap(),
        claim.actual_vout.unwrap()
    );

    // ========================================================================
    // Step 8: Now confirm transaction and verify everything still works
    // ========================================================================
    println!("\n📋 Step 8: Mining blocks to confirm transaction");

    env.wait_for_confirmation(&transfer_response.bitcoin_txid, 1)
        .await
        .expect("Failed to confirm transaction");

    println!("✓ Transaction confirmed");

    // Sync and verify balance still correct
    bob.sync_wallet().await.expect("Failed to sync Bob wallet");

    verify_balance_with_retry(&mut bob, &asset_info.contract_id, 2_500, 5)
        .await
        .expect("Bob balance mismatch after confirmation");

    println!("✓ Balance still correct after confirmation");

    println!("\n✅ Claim before confirmation test passed!");
    println!("   This demonstrates that with the production fix, claims work immediately");
    println!("   using actual UTXO from consignment, even before Bitcoin confirmation.");
}
