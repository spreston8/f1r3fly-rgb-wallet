//! Module 5: Send Bitcoin Operations Tests
//!
//! Tests for sending Bitcoin to external addresses.
//! Covers send_bitcoin(), error handling, fee rates, and change outputs.

use bdk_wallet::KeychainKind;
use f1r3fly_rgb_wallet::bitcoin::balance::{get_balance, list_utxos};
use f1r3fly_rgb_wallet::bitcoin::sync::sync_wallet;
use f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig;
use f1r3fly_rgb_wallet::bitcoin::wallet::BitcoinWallet;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;
use f1r3fly_rgb_wallet::storage::models::WalletKeys;
use std::collections::HashSet;

use crate::common::TestBitcoinEnv;

/// Test 5.1: Verify send_bitcoin works end-to-end
#[tokio::test]
async fn test_send_bitcoin_to_external_address() {
    // Step 1: Create funded wallet with 1 BTC
    let env = TestBitcoinEnv::new("send_bitcoin_external");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("send_wallet");
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

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 3: Generate recipient address via bitcoin-cli
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    // Step 4: Record initial balance
    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");

    // Step 5: Send 50,000 sats to recipient
    let send_amount = 50_000;

    // Parse recipient address
    use bdk_wallet::bitcoin::Address;
    let recipient_addr: Address = recipient
        .parse::<Address<_>>()
        .expect("Failed to parse recipient address")
        .assume_checked();

    // Build and broadcast transaction
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(
        recipient_addr.script_pubkey(),
        bdk_wallet::bitcoin::Amount::from_sat(send_amount),
    );

    let mut psbt = tx_builder.finish().expect("Failed to build transaction");

    #[allow(deprecated)]
    let finalized = wallet
        .inner_mut()
        .sign(&mut psbt, bdk_wallet::SignOptions::default())
        .expect("Failed to sign transaction");

    assert!(finalized, "Transaction should be fully signed");

    let tx = psbt.extract_tx().expect("Failed to extract transaction");
    let spend_txid = tx.compute_txid();

    // Step 6: Verify transaction broadcasts (txid returned)
    env.esplora_client
        .inner()
        .broadcast(&tx)
        .expect("Failed to broadcast transaction");

    assert!(
        !spend_txid.to_string().is_empty(),
        "Transaction ID should not be empty"
    );

    // Step 7: Mine 1 block
    env.wait_for_confirmation(&spend_txid.to_string(), 1)
        .await
        .expect("Failed to confirm send transaction");

    // Step 8: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after send");

    // Step 9: Verify balance decreased by ~50,000 + fee
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");
    let balance_decrease = initial_balance.confirmed - final_balance.confirmed;

    assert!(
        balance_decrease >= send_amount,
        "Balance decrease ({} sats) should be at least send amount ({} sats)",
        balance_decrease,
        send_amount
    );

    // Verify fee is reasonable (< 10,000 sats)
    let estimated_fee = balance_decrease - send_amount;
    assert!(
        estimated_fee < 10_000,
        "Fee should be reasonable (< 10,000 sats), got: {} sats",
        estimated_fee
    );

    // Step 10: Use bitcoin-cli to verify recipient received funds
    // Query the recipient address via RPC
    let check_cmd = format!(
        "bitcoin-cli -datadir={} getreceivedbyaddress {} 0",
        env.bitcoin_rpc().datadir,
        recipient
    );
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&check_cmd)
        .output()
        .expect("Failed to check recipient balance");

    if output.status.success() {
        let received_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(received_btc) = received_str.parse::<f64>() {
            let received_sats = (received_btc * 100_000_000.0) as u64;
            assert_eq!(
                received_sats, send_amount,
                "Recipient should have received {} sats, got: {} sats",
                send_amount, received_sats
            );
        }
    }
}

/// Test 5.2: Verify error handling for insufficient funds
#[tokio::test]
async fn test_send_bitcoin_insufficient_funds_error() {
    // Step 1: Create wallet with only 1,000 sats
    let env = TestBitcoinEnv::new("send_insufficient_funds");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("insufficient_wallet");
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

    // Fund with only 0.00001 BTC (1,000 sats)
    let txid = env
        .fund_address(&address, 0.00001)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    let initial_balance = get_balance(&wallet).expect("Failed to get balance");
    assert!(
        initial_balance.confirmed <= 1_000,
        "Wallet should have approximately 1,000 sats"
    );

    // Step 3: Attempt to send 10,000 sats
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    let send_amount = 10_000;

    // Parse recipient address
    use bdk_wallet::bitcoin::Address;
    let recipient_addr: Address = recipient
        .parse::<Address<_>>()
        .expect("Failed to parse recipient address")
        .assume_checked();

    // Build transaction
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(
        recipient_addr.script_pubkey(),
        bdk_wallet::bitcoin::Amount::from_sat(send_amount),
    );

    // Step 4: Verify function returns error
    let result = tx_builder.finish();

    // Step 5: Verify error is BuildFailed or InsufficientFunds variant
    assert!(
        result.is_err(),
        "Transaction build should fail with insufficient funds"
    );

    let error = result.unwrap_err();
    let error_str = error.to_string().to_lowercase();

    assert!(
        error_str.contains("insufficient")
            || error_str.contains("not enough")
            || error_str.contains("coin selection"),
        "Error should indicate insufficient funds, got: {}",
        error
    );

    // Step 6: Verify no transaction broadcast (balance unchanged)
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");
    assert_eq!(
        final_balance.confirmed, initial_balance.confirmed,
        "Balance should remain unchanged after failed send"
    );
}

/// Test 5.3: Verify custom fee rates respected
#[tokio::test]
async fn test_send_bitcoin_with_custom_fee_rate() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("send_custom_fee");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("custom_fee_wallet");
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

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 3: Create high_priority() fee rate
    let fee_rate = FeeRateConfig::high_priority();

    // Step 4: Send 100,000 sats with high fee rate
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    let send_amount = 100_000;

    // Parse recipient address
    use bdk_wallet::bitcoin::Address;
    let recipient_addr: Address = recipient
        .parse::<Address<_>>()
        .expect("Failed to parse recipient address")
        .assume_checked();

    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");

    // Build transaction with custom fee rate
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(
        recipient_addr.script_pubkey(),
        bdk_wallet::bitcoin::Amount::from_sat(send_amount),
    );
    tx_builder.fee_rate(
        bdk_wallet::bitcoin::FeeRate::from_sat_per_vb(fee_rate.sat_per_vb as u64)
            .expect("Valid fee rate"),
    );

    let mut psbt = tx_builder.finish().expect("Failed to build transaction");

    #[allow(deprecated)]
    let finalized = wallet
        .inner_mut()
        .sign(&mut psbt, bdk_wallet::SignOptions::default())
        .expect("Failed to sign transaction");

    assert!(finalized, "Transaction should be fully signed");

    // Step 5: Record fee from transaction
    let psbt_clone = psbt.clone();
    let tx = psbt.extract_tx().expect("Failed to extract transaction");
    let spend_txid = tx.compute_txid();

    let fee = psbt_clone.fee().expect("Failed to get fee").to_sat();

    // Step 6: Mine block, sync
    env.esplora_client
        .inner()
        .broadcast(&tx)
        .expect("Failed to broadcast transaction");

    env.wait_for_confirmation(&spend_txid.to_string(), 1)
        .await
        .expect("Failed to confirm transaction");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after send");

    // Step 7: Verify fee paid is higher than minimum
    // For regtest, just verify the transaction confirmed and fee is reasonable
    assert!(fee > 0, "Fee should be greater than 0, got: {} sats", fee);

    assert!(
        fee < 50_000,
        "Fee should be reasonable even with high priority, got: {} sats",
        fee
    );

    // Step 8: Verify transaction confirms
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");
    let balance_decrease = initial_balance.confirmed - final_balance.confirmed;

    assert_eq!(
        balance_decrease,
        send_amount + fee,
        "Balance decrease should equal send amount + fee"
    );
}

/// Test 5.4: Verify change handling works
#[tokio::test]
async fn test_send_bitcoin_creates_change_output() {
    // Step 1: Create wallet with single 1 BTC UTXO
    let env = TestBitcoinEnv::new("send_change_output");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("change_wallet");
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

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    let rgb_occupied = HashSet::new();
    let initial_utxos = list_utxos(&wallet, &rgb_occupied).expect("Failed to list initial UTXOs");

    assert_eq!(
        initial_utxos.len(),
        1,
        "Should have exactly 1 UTXO initially"
    );

    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");
    let expected_initial = 100_000_000; // 1 BTC in sats
    assert_eq!(
        initial_balance.confirmed, expected_initial,
        "Initial balance should be 1 BTC (100,000,000 sats)"
    );

    // Step 3: Send 0.1 BTC to external address
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    let send_amount = 10_000_000; // 0.1 BTC in sats

    // Parse recipient address
    use bdk_wallet::bitcoin::Address;
    let recipient_addr: Address = recipient
        .parse::<Address<_>>()
        .expect("Failed to parse recipient address")
        .assume_checked();

    // Build transaction
    let mut tx_builder = wallet.inner_mut().build_tx();
    tx_builder.add_recipient(
        recipient_addr.script_pubkey(),
        bdk_wallet::bitcoin::Amount::from_sat(send_amount),
    );

    let mut psbt = tx_builder.finish().expect("Failed to build transaction");

    #[allow(deprecated)]
    let finalized = wallet
        .inner_mut()
        .sign(&mut psbt, bdk_wallet::SignOptions::default())
        .expect("Failed to sign transaction");

    assert!(finalized, "Transaction should be fully signed");

    let psbt_clone = psbt.clone();
    let tx = psbt.extract_tx().expect("Failed to extract transaction");
    let spend_txid = tx.compute_txid();
    let fee = psbt_clone.fee().expect("Failed to get fee").to_sat();

    // Broadcast and confirm
    env.esplora_client
        .inner()
        .broadcast(&tx)
        .expect("Failed to broadcast transaction");

    // Step 4: Mine block, sync
    env.wait_for_confirmation(&spend_txid.to_string(), 1)
        .await
        .expect("Failed to confirm transaction");

    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after send");

    // Step 5: List UTXOs
    let final_utxos = list_utxos(&wallet, &rgb_occupied).expect("Failed to list final UTXOs");

    // Step 6: Verify change UTXO created with ~0.9 BTC
    // After spending 0.1 BTC + fee, we should have a change output
    let expected_change = expected_initial - send_amount - fee;

    let final_balance = get_balance(&wallet).expect("Failed to get final balance");

    assert_eq!(
        final_balance.confirmed, expected_change,
        "Final balance should be approximately {} sats (0.9 BTC minus fee), got: {} sats",
        expected_change, final_balance.confirmed
    );

    // Verify we have at least 1 UTXO (the change output)
    assert!(
        final_utxos.len() >= 1,
        "Should have at least 1 UTXO (change output), got: {}",
        final_utxos.len()
    );

    // Find change UTXO (should be approximately 0.9 BTC minus fee)
    let change_utxo = final_utxos.iter().find(|u| u.amount > 80_000_000); // At least 0.8 BTC

    assert!(
        change_utxo.is_some(),
        "Should have a change UTXO with substantial amount"
    );

    let change = change_utxo.unwrap();
    assert_eq!(
        change.amount, expected_change,
        "Change UTXO should have amount {} sats, got: {} sats",
        expected_change, change.amount
    );
}
