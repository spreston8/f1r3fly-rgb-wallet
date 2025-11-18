//! Module 4: UTXO Operations Tests
//!
//! Tests for UTXO creation, unlocking, fee rate handling, and RGB UTXO marking.
//! Covers create_utxo(), unlock_utxo(), and fee rate estimation.

use bdk_wallet::KeychainKind;
use f1r3fly_rgb_wallet::bitcoin::balance::{get_balance, list_utxos};
use f1r3fly_rgb_wallet::bitcoin::sync::sync_wallet;
use f1r3fly_rgb_wallet::bitcoin::utxo::{
    create_utxo, get_recommended_fee_rates, unlock_utxo, FeeRateConfig,
};
use f1r3fly_rgb_wallet::bitcoin::wallet::BitcoinWallet;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;
use f1r3fly_rgb_wallet::storage::models::WalletKeys;
use std::collections::HashSet;

use crate::common::TestBitcoinEnv;

/// Test 4.1: Verify create_utxo creates exact UTXO amount
#[tokio::test]
async fn test_create_utxo_self_send_with_specific_amount() {
    // Step 1: Create funded wallet with sufficient balance (1 BTC)
    let env = TestBitcoinEnv::new("create_utxo_exact_amount");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("utxo_creation_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Fund with 1 BTC
    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 3: Record initial balance
    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");

    // Step 4: Create UTXO of 10,000 sats with medium fee rate
    let target_amount = 10_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &fee_rate,
        None,
        false,
    )
    .expect("Failed to create UTXO");

    // Step 5: Verify UtxoOperationResult returned with txid
    assert!(
        !result.txid.to_string().is_empty(),
        "Transaction ID should not be empty"
    );
    assert_eq!(
        result.amount, target_amount,
        "Result should show target amount of {} sats, got: {} sats",
        target_amount, result.amount
    );

    // Step 6: Mine 1 block
    env.wait_for_confirmation(&result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm UTXO creation transaction");

    // Step 7: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after UTXO creation");

    // Step 8: List UTXOs
    let rgb_occupied = HashSet::new();
    let after_utxos =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after creation");

    // Step 9: Find newly created UTXO
    let new_utxo = after_utxos
        .iter()
        .find(|u| u.outpoint == result.outpoint)
        .expect("Created UTXO should be in list");

    // Step 10: Verify amount = 10,000 sats exactly
    assert_eq!(
        new_utxo.amount, target_amount,
        "Created UTXO should have exact amount of {} sats, got: {} sats",
        target_amount, new_utxo.amount
    );

    // Step 11: Verify total balance decreased by (10,000 + fee)
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");
    let balance_decrease = initial_balance.confirmed - final_balance.confirmed;

    assert!(
        balance_decrease >= target_amount,
        "Balance decrease ({} sats) should be at least target amount ({} sats)",
        balance_decrease,
        target_amount
    );

    // Verify fee was reasonable (should be less than 10,000 sats for a simple tx)
    let estimated_fee = balance_decrease - target_amount;
    assert!(
        estimated_fee < 10_000,
        "Fee should be reasonable (< 10,000 sats), got: {} sats",
        estimated_fee
    );
}

/// Test 4.2: Verify different fee rates work correctly
#[tokio::test]
async fn test_create_utxo_with_different_fee_rates() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("create_utxo_fee_rates");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("fee_rate_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Fund with 2 BTC
    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 2.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    let target_amount = 50_000;

    // Step 2: Create UTXO with low_priority() fee rate
    let low_fee_config = FeeRateConfig::low_priority();
    let low_result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &low_fee_config,
        None,
        false,
    )
    .expect("Failed to create UTXO with low fee");

    // Step 3: Record fee from result
    let low_fee = low_result.fee;

    // Step 4: Mine block, sync
    env.wait_for_confirmation(&low_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm low fee transaction");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after low fee");

    // Step 5: Create UTXO with medium_priority() fee rate
    let medium_fee_config = FeeRateConfig::medium_priority();
    let medium_result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &medium_fee_config,
        None,
        false,
    )
    .expect("Failed to create UTXO with medium fee");

    // Step 6: Record fee from result
    let medium_fee = medium_result.fee;

    // Step 7: Mine block, sync
    env.wait_for_confirmation(&medium_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm medium fee transaction");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after medium fee");

    // Step 8: Create UTXO with high_priority() fee rate
    let high_fee_config = FeeRateConfig::high_priority();
    let high_result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &high_fee_config,
        None,
        false,
    )
    .expect("Failed to create UTXO with high fee");

    // Step 9: Record fee from result
    let high_fee = high_result.fee;

    // Step 10: Verify high_fee >= medium_fee >= low_fee
    assert!(
        high_fee >= medium_fee,
        "High fee ({} sats) should be >= medium fee ({} sats)",
        high_fee,
        medium_fee
    );
    assert!(
        medium_fee >= low_fee,
        "Medium fee ({} sats) should be >= low fee ({} sats)",
        medium_fee,
        low_fee
    );

    // Verify all transactions produced valid results
    // Note: Previously created UTXOs might have been spent as inputs for later transactions
    // The key test is that all three operations succeeded with different fee rates
    assert!(
        low_result.amount == target_amount,
        "Low fee UTXO should have target amount"
    );
    assert!(
        medium_result.amount == target_amount,
        "Medium fee UTXO should have target amount"
    );
    assert!(
        high_result.amount == target_amount,
        "High fee UTXO should have target amount"
    );

    // Verify all transactions were broadcast successfully
    assert!(
        !low_result.txid.to_string().is_empty(),
        "Low fee transaction should have valid txid"
    );
    assert!(
        !medium_result.txid.to_string().is_empty(),
        "Medium fee transaction should have valid txid"
    );
    assert!(
        !high_result.txid.to_string().is_empty(),
        "High fee transaction should have valid txid"
    );
}

/// Test 4.3: Verify RGB marking during UTXO creation
#[tokio::test]
async fn test_create_utxo_marks_rgb_occupied_when_requested() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("create_utxo_rgb_marking");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("rgb_marking_utxo_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 2: Create empty RGB-occupied set
    let mut rgb_occupied = HashSet::new();

    // Step 3: Create UTXO with RGB set, which will mark it as RGB-occupied
    let target_amount = 25_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &fee_rate,
        Some(&mut rgb_occupied),
        true,
    )
    .expect("Failed to create RGB-occupied UTXO");

    // Step 4: Verify outpoint in RGB-occupied set
    assert!(
        rgb_occupied.contains(&result.outpoint),
        "Created UTXO outpoint should be in RGB-occupied set immediately"
    );

    // Step 5: Mine block, sync
    env.wait_for_confirmation(&result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm RGB UTXO creation");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after RGB UTXO creation");

    // Step 6: List UTXOs with RGB set
    let utxos = list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs");

    // Step 7: Verify new UTXO has is_rgb_occupied=true
    let created_utxo = utxos
        .iter()
        .find(|u| u.outpoint == result.outpoint)
        .expect("Created UTXO should be in list");

    assert!(
        created_utxo.is_rgb_occupied,
        "Created UTXO should be marked as RGB-occupied"
    );
}

/// Test 4.4: Verify unlock_utxo works correctly
#[tokio::test]
async fn test_unlock_utxo_spends_back_to_self() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("unlock_utxo_test");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("unlock_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 2: Create specific UTXO of 50,000 sats
    let target_amount = 50_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let create_result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &fee_rate,
        None,
        false,
    )
    .expect("Failed to create UTXO");

    // Step 3: Mine block, sync
    env.wait_for_confirmation(&create_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm UTXO creation");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after creation");

    // Step 4: Record UTXO details (outpoint, amount)
    let original_outpoint = create_result.outpoint;
    let original_amount = create_result.amount;

    // Verify UTXO exists
    let rgb_occupied = HashSet::new();
    let utxos_before =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs before unlock");

    let original_utxo = utxos_before
        .iter()
        .find(|u| u.outpoint == original_outpoint)
        .expect("Original UTXO should exist");

    assert_eq!(
        original_utxo.amount, original_amount,
        "Original UTXO should have amount {} sats",
        original_amount
    );

    // Step 5: Unlock the UTXO with medium fee rate
    let unlock_result = unlock_utxo(
        &mut wallet,
        &env.esplora_client,
        original_outpoint,
        &fee_rate,
        None,
    )
    .expect("Failed to unlock UTXO");

    // Step 6: Verify UtxoOperationResult returned
    assert!(
        !unlock_result.txid.to_string().is_empty(),
        "Unlock transaction ID should not be empty"
    );

    // Step 7: Mine block, sync
    env.wait_for_confirmation(&unlock_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm unlock transaction");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after unlock");

    // Step 8: List UTXOs
    let utxos_after =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after unlock");

    // Step 9: Verify original UTXO gone (spent)
    let original_still_exists = utxos_after.iter().any(|u| u.outpoint == original_outpoint);

    assert!(
        !original_still_exists,
        "Original UTXO should be spent and not in list"
    );

    // Step 10: Verify new UTXO exists with amount ≈ 50,000 - fee
    let new_utxo = utxos_after
        .iter()
        .find(|u| u.outpoint == unlock_result.outpoint)
        .expect("New UTXO from unlock should exist");

    assert_eq!(
        new_utxo.amount, unlock_result.amount,
        "New UTXO amount should match result amount"
    );

    // Verify amount is approximately original minus fee
    let expected_min = original_amount - 10_000; // Allow up to 10k sats for fee
    assert!(
        new_utxo.amount >= expected_min,
        "New UTXO amount ({} sats) should be at least {} sats (original {} - max fee 10k)",
        new_utxo.amount,
        expected_min,
        original_amount
    );
    assert!(
        new_utxo.amount < original_amount,
        "New UTXO amount ({} sats) should be less than original ({} sats) due to fee",
        new_utxo.amount,
        original_amount
    );
}

/// Test 4.5: Verify RGB flag removed when unlocking
#[tokio::test]
async fn test_unlock_utxo_removes_rgb_occupied_flag() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("unlock_rgb_removal");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("unlock_rgb_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 2: Create RGB-occupied set
    let mut rgb_occupied = HashSet::new();

    // Step 3: Create UTXO with RGB set to mark it as RGB-occupied
    let target_amount = 30_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let create_result = create_utxo(
        &mut wallet,
        &env.esplora_client,
        target_amount,
        &fee_rate,
        Some(&mut rgb_occupied),
        true,
    )
    .expect("Failed to create RGB UTXO");

    // Step 4: Mine block, sync
    env.wait_for_confirmation(&create_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm RGB UTXO creation");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after creation");

    // Step 5: Verify UTXO in RGB set
    assert!(
        rgb_occupied.contains(&create_result.outpoint),
        "UTXO should be in RGB-occupied set"
    );

    let utxos_before =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs before unlock");

    let rgb_utxo = utxos_before
        .iter()
        .find(|u| u.outpoint == create_result.outpoint)
        .expect("RGB UTXO should exist");

    assert!(
        rgb_utxo.is_rgb_occupied,
        "UTXO should be marked as RGB-occupied"
    );

    // Step 6: Unlock UTXO, passing RGB set to remove the RGB flag
    let unlock_result = unlock_utxo(
        &mut wallet,
        &env.esplora_client,
        create_result.outpoint,
        &fee_rate,
        Some(&mut rgb_occupied),
    )
    .expect("Failed to unlock RGB UTXO");

    // Step 7: Verify original outpoint removed from RGB set
    assert!(
        !rgb_occupied.contains(&create_result.outpoint),
        "Original UTXO outpoint should be removed from RGB-occupied set"
    );

    // Step 8: Mine block, sync
    env.wait_for_confirmation(&unlock_result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm unlock transaction");
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after unlock");

    // Step 9: List UTXOs
    let utxos_after =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after unlock");

    // Step 10: Verify new UTXO not marked as RGB-occupied
    let new_utxo = utxos_after
        .iter()
        .find(|u| u.outpoint == unlock_result.outpoint)
        .expect("New UTXO should exist");

    assert!(
        !new_utxo.is_rgb_occupied,
        "New UTXO should not be marked as RGB-occupied"
    );
}

/// Test 4.6: Verify fee rate estimation works
#[tokio::test]
async fn test_get_recommended_fee_rates_from_esplora() {
    // Step 1: Create EsploraClient for regtest
    let env = TestBitcoinEnv::new("fee_rate_estimation");

    // Step 2: Call get_recommended_fee_rates()
    let fee_rates = get_recommended_fee_rates(&env.esplora_client)
        .expect("Failed to get recommended fee rates");

    // Step 3: Verify returns (low, medium, high)
    let (low, medium, high) = fee_rates;

    // Step 4: Verify low <= medium <= high
    assert!(
        low.sat_per_vb <= medium.sat_per_vb,
        "Low fee rate ({} sat/vB) should be <= medium ({} sat/vB)",
        low.sat_per_vb,
        medium.sat_per_vb
    );
    assert!(
        medium.sat_per_vb <= high.sat_per_vb,
        "Medium fee rate ({} sat/vB) should be <= high ({} sat/vB)",
        medium.sat_per_vb,
        high.sat_per_vb
    );

    // Step 5: Verify all > 0
    assert!(
        low.sat_per_vb > 0.0,
        "Low fee rate should be > 0, got: {} sat/vB",
        low.sat_per_vb
    );
    assert!(
        medium.sat_per_vb > 0.0,
        "Medium fee rate should be > 0, got: {} sat/vB",
        medium.sat_per_vb
    );
    assert!(
        high.sat_per_vb > 0.0,
        "High fee rate should be > 0, got: {} sat/vB",
        high.sat_per_vb
    );

    // Step 6: Verify reasonable values (e.g., < 1000 sat/vB)
    assert!(
        low.sat_per_vb < 1000.0,
        "Low fee rate should be < 1000 sat/vB, got: {} sat/vB",
        low.sat_per_vb
    );
    assert!(
        medium.sat_per_vb < 1000.0,
        "Medium fee rate should be < 1000 sat/vB, got: {} sat/vB",
        medium.sat_per_vb
    );
    assert!(
        high.sat_per_vb < 1000.0,
        "High fee rate should be < 1000 sat/vB, got: {} sat/vB",
        high.sat_per_vb
    );
}
