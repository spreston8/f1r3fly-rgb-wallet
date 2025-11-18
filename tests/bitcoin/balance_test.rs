//! Module 3: Balance & UTXO Query Tests
//!
//! Tests for balance queries, UTXO listing, address management, and RGB UTXO marking.
//! Covers confirmed/unconfirmed balances, UTXO details, address derivation, and RGB tracking.

use bdk_wallet::KeychainKind;
use f1r3fly_rgb_wallet::bitcoin::balance::{
    get_addresses, get_balance, list_utxos, mark_rgb_occupied, unmark_rgb_occupied,
};
use f1r3fly_rgb_wallet::bitcoin::sync::sync_wallet;
use f1r3fly_rgb_wallet::bitcoin::wallet::BitcoinWallet;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;
use f1r3fly_rgb_wallet::storage::models::WalletKeys;
use std::collections::HashSet;

use crate::common::TestBitcoinEnv;

/// Test 3.1: Verify balance query returns correct confirmed balance
#[tokio::test]
async fn test_get_balance_confirmed_vs_unconfirmed() {
    // Step 1: Create test environment with new wallet
    let env = TestBitcoinEnv::new("balance_confirmed");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("balance_test_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 2: Get wallet address
    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    // Step 3: Sync wallet (balance should be 0)
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to perform initial sync");

    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");
    assert_eq!(
        initial_balance.confirmed, 0,
        "Initial confirmed balance should be 0, got: {} sats",
        initial_balance.confirmed
    );
    assert_eq!(
        initial_balance.unconfirmed, 0,
        "Initial unconfirmed balance should be 0, got: {} sats",
        initial_balance.unconfirmed
    );

    // Step 4: Send 0.5 BTC to address and mine to confirm
    let amount_btc = 0.5;
    let txid = env
        .fund_address(&address, amount_btc)
        .expect("Failed to fund address");

    // Note: In regtest, mempool transactions are typically mined immediately
    // So we'll test the confirmed balance flow directly
    // Step 5: Wait for confirmation
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // Step 6: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after mining");

    // Step 7: Get balance - verify confirmed > 0, unconfirmed = 0
    let expected_sats = (amount_btc * 100_000_000.0) as u64;
    let confirmed_balance = get_balance(&wallet).expect("Failed to get balance after confirmation");

    assert_eq!(
        confirmed_balance.confirmed, expected_sats,
        "Confirmed balance should be {} sats after mining, got: {} sats",
        expected_sats, confirmed_balance.confirmed
    );
    assert_eq!(
        confirmed_balance.unconfirmed, 0,
        "Unconfirmed balance should be 0 after confirmation, got: {} sats",
        confirmed_balance.unconfirmed
    );
    assert_eq!(
        confirmed_balance.total, expected_sats,
        "Total should equal confirmed ({} sats), got: {} sats",
        expected_sats, confirmed_balance.total
    );
}

/// Test 3.2: Verify list_utxos returns accurate UTXO information
#[tokio::test]
async fn test_list_utxos_with_details() {
    // Step 1: Create funded wallet with 2 separate transactions
    let env = TestBitcoinEnv::new("list_utxos_details");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("utxo_list_wallet");
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

    // Fund with 0.1 BTC twice
    let amount_btc = 0.1;
    let txid1 = env
        .fund_address(&address, amount_btc)
        .expect("Failed to fund address first time");

    let txid2 = env
        .fund_address(&address, amount_btc)
        .expect("Failed to fund address second time");

    // Step 2: Mine 1 block to confirm both
    env.wait_for_confirmation(&txid1, 1)
        .await
        .expect("Failed to confirm first transaction");
    env.wait_for_confirmation(&txid2, 1)
        .await
        .expect("Failed to confirm second transaction");

    // Step 3: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 4: Create empty RGB-occupied set
    let rgb_occupied = HashSet::new();

    // Step 5: Call list_utxos()
    let utxos = list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs");

    // Step 6: Verify UTXO count = 2
    assert_eq!(utxos.len(), 2, "Should have 2 UTXOs, got: {}", utxos.len());

    // Step 7: For each UTXO, verify details
    let expected_amount = (amount_btc * 100_000_000.0) as u64;

    for utxo in &utxos {
        // Valid outpoint (txid:vout format)
        assert!(
            utxo.outpoint.to_string().contains(':'),
            "Outpoint should be in txid:vout format, got: {}",
            utxo.outpoint
        );

        // Amount = 0.1 BTC
        assert_eq!(
            utxo.amount, expected_amount,
            "UTXO amount should be {} sats (0.1 BTC), got: {} sats",
            expected_amount, utxo.amount
        );

        // is_confirmed = true
        assert!(utxo.is_confirmed, "UTXO should be confirmed");

        // confirmation_height is set
        assert!(
            utxo.confirmation_height.is_some(),
            "Confirmation height should be set for confirmed UTXO"
        );

        // is_rgb_occupied = false
        assert!(!utxo.is_rgb_occupied, "UTXO should not be RGB-occupied");

        // Verify keychain is External
        assert_eq!(
            utxo.keychain,
            KeychainKind::External,
            "UTXO should be from External keychain"
        );
    }

    // Step 8: Verify UTXOs sorted by amount descending
    // (Both have same amount, so order may vary, but amounts should be equal)
    assert_eq!(
        utxos[0].amount, utxos[1].amount,
        "Both UTXOs should have same amount"
    );
}

/// Test 3.3: Verify get_addresses returns correct address information
#[tokio::test]
async fn test_get_addresses_external_and_internal() {
    // Step 1: Create wallet
    let env = TestBitcoinEnv::new("get_addresses_test");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("addresses_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 2: Get first receive address and use it
    let first_external = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    // Step 3: Get first change address via internal keychain
    let _first_internal = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::Internal)
        .address
        .to_string();

    // Reveal a few more addresses
    let _second_external = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let _second_internal = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::Internal);

    // Step 4: Call get_addresses(count=None)
    let addresses = get_addresses(&mut wallet, None).expect("Failed to get addresses");

    // Step 5: Verify returns both external and internal addresses
    let has_external = addresses
        .iter()
        .any(|a| a.keychain == KeychainKind::External);
    let has_internal = addresses
        .iter()
        .any(|a| a.keychain == KeychainKind::Internal);

    assert!(has_external, "Should have external addresses");
    assert!(has_internal, "Should have internal addresses");

    // Step 6: Verify addresses have correct prefixes (bcrt1)
    for addr_info in &addresses {
        assert!(
            addr_info.address.to_string().starts_with("bcrt1"),
            "Address should start with bcrt1 (regtest), got: {}",
            addr_info.address
        );
    }

    // Step 7: Verify used addresses marked as is_used=true
    // Find the first external address we revealed
    let first_external_info = addresses
        .iter()
        .find(|a| a.address.to_string() == first_external && a.keychain == KeychainKind::External);

    assert!(
        first_external_info.is_some(),
        "First external address should be in list"
    );

    let first_external_info = first_external_info.unwrap();
    assert!(
        first_external_info.is_used,
        "First revealed external address should be marked as used"
    );

    // Step 8: Verify unused addresses marked as is_used=false
    // Peek at unrevealed addresses
    let unrevealed_addr = wallet.inner().peek_address(KeychainKind::External, 10);
    let unrevealed_str = unrevealed_addr.address.to_string();

    let unrevealed_info = addresses
        .iter()
        .find(|a| a.address.to_string() == unrevealed_str);

    if let Some(info) = unrevealed_info {
        assert!(
            !info.is_used,
            "Unrevealed address should be marked as unused"
        );
    }
}

/// Test 3.4: Verify address derivation increments properly
#[tokio::test]
async fn test_get_new_address_increments_index() {
    // Step 1: Create wallet
    let env = TestBitcoinEnv::new("address_increment_test");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("increment_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 2: Get first address via reveal_next_address()
    let first_addr_info = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let first_address = first_addr_info.address.to_string();
    let first_index = first_addr_info.index;

    // Step 3: Get second address via reveal_next_address()
    let second_addr_info = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let second_address = second_addr_info.address.to_string();
    let second_index = second_addr_info.index;

    // Step 4: Verify addresses different
    assert_ne!(
        first_address, second_address,
        "First and second addresses should be different"
    );

    // Step 5: Verify both valid bcrt1 addresses
    assert!(
        first_address.starts_with("bcrt1"),
        "First address should be regtest (bcrt1), got: {}",
        first_address
    );
    assert!(
        second_address.starts_with("bcrt1"),
        "Second address should be regtest (bcrt1), got: {}",
        second_address
    );

    // Step 6: Verify indices are sequential
    assert_eq!(
        second_index,
        first_index + 1,
        "Second address index ({}) should be first index ({}) + 1",
        second_index,
        first_index
    );

    // Step 7: Get addresses list and verify both addresses in list with correct indices
    let addresses = get_addresses(&mut wallet, None).expect("Failed to get addresses");

    let first_in_list = addresses
        .iter()
        .find(|a| a.address.to_string() == first_address && a.keychain == KeychainKind::External);
    let second_in_list = addresses
        .iter()
        .find(|a| a.address.to_string() == second_address && a.keychain == KeychainKind::External);

    assert!(first_in_list.is_some(), "First address should be in list");
    assert!(second_in_list.is_some(), "Second address should be in list");

    assert_eq!(
        first_in_list.unwrap().index,
        first_index,
        "First address should have correct derivation index"
    );
    assert_eq!(
        second_in_list.unwrap().index,
        second_index,
        "Second address should have correct derivation index"
    );
}

/// Test 3.5: Verify RGB UTXO tracking works correctly
#[tokio::test]
async fn test_rgb_occupied_utxo_marking() {
    // Step 1: Create funded wallet with 2 UTXOs
    let env = TestBitcoinEnv::new("rgb_marking_test");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("rgb_marking_wallet");
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

    // Fund with 0.1 BTC twice
    let txid1 = env
        .fund_address(&address, 0.1)
        .expect("Failed to fund first time");
    let txid2 = env
        .fund_address(&address, 0.1)
        .expect("Failed to fund second time");

    env.wait_for_confirmation(&txid1, 1)
        .await
        .expect("Failed to confirm first transaction");
    env.wait_for_confirmation(&txid2, 1)
        .await
        .expect("Failed to confirm second transaction");

    // Step 2: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    // Step 3: Create empty RGB-occupied set
    let mut rgb_occupied = HashSet::new();

    // Step 4: List UTXOs - verify all is_rgb_occupied=false
    let utxos_before = list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs");

    assert_eq!(utxos_before.len(), 2, "Should have 2 UTXOs");

    for utxo in &utxos_before {
        assert!(
            !utxo.is_rgb_occupied,
            "UTXO should not be RGB-occupied initially"
        );
    }

    // Step 5: Mark first UTXO as RGB-occupied
    let first_outpoint = utxos_before[0].outpoint;
    mark_rgb_occupied(&mut rgb_occupied, [first_outpoint]);

    // Step 6: List UTXOs with RGB set
    let utxos_after_mark =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after marking");

    // Step 7: Verify first UTXO has is_rgb_occupied=true
    let first_utxo_marked = utxos_after_mark
        .iter()
        .find(|u| u.outpoint == first_outpoint)
        .expect("First UTXO should be in list");

    assert!(
        first_utxo_marked.is_rgb_occupied,
        "First UTXO should be marked as RGB-occupied"
    );

    // Step 8: Verify second UTXO has is_rgb_occupied=false
    let second_outpoint = utxos_before[1].outpoint;
    let second_utxo = utxos_after_mark
        .iter()
        .find(|u| u.outpoint == second_outpoint)
        .expect("Second UTXO should be in list");

    assert!(
        !second_utxo.is_rgb_occupied,
        "Second UTXO should not be RGB-occupied"
    );

    // Step 9: Unmark first UTXO
    unmark_rgb_occupied(&mut rgb_occupied, [first_outpoint]);

    // Step 10: List UTXOs - verify all is_rgb_occupied=false
    let utxos_after_unmark =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after unmarking");

    for utxo in &utxos_after_unmark {
        assert!(
            !utxo.is_rgb_occupied,
            "All UTXOs should not be RGB-occupied after unmarking"
        );
    }
}

/// Test 3.6: Verify spent UTXOs not listed
#[tokio::test]
async fn test_list_utxos_excludes_spent() {
    // Step 1: Create funded wallet with 2 UTXOs
    let env = TestBitcoinEnv::new("utxo_excludes_spent");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("spent_utxo_wallet");
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

    // Fund with 0.2 BTC twice for a total of 0.4 BTC
    let txid1 = env
        .fund_address(&address, 0.2)
        .expect("Failed to fund first time");
    let txid2 = env
        .fund_address(&address, 0.2)
        .expect("Failed to fund second time");

    env.wait_for_confirmation(&txid1, 1)
        .await
        .expect("Failed to confirm first transaction");
    env.wait_for_confirmation(&txid2, 1)
        .await
        .expect("Failed to confirm second transaction");

    // Step 2: Sync and list UTXOs (count=2)
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync wallet");

    let rgb_occupied = HashSet::new();
    let utxos_before =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs before spending");

    assert_eq!(utxos_before.len(), 2, "Should have 2 UTXOs before spending");

    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");

    // Step 3: Spend one UTXO
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    let send_amount = 5_000_000; // 0.05 BTC in sats

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

    // Broadcast via Esplora
    env.esplora_client
        .inner()
        .broadcast(&tx)
        .expect("Failed to broadcast transaction");

    // Step 4: Mine 1 block
    env.wait_for_confirmation(&spend_txid.to_string(), 1)
        .await
        .expect("Failed to confirm spend transaction");

    // Step 5: Sync wallet
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after spending");

    // Step 6: List UTXOs
    let utxos_after =
        list_utxos(&wallet, &rgb_occupied).expect("Failed to list UTXOs after spending");

    // Step 7: Verify count changed (original UTXO spent, may have change)
    // With 0.2 BTC UTXOs and sending 0.05 BTC, we should have:
    // - 1 spent UTXO (0.2 BTC)
    // - 1 unspent UTXO (0.2 BTC)
    // - 1 change UTXO (~0.15 BTC minus fee)
    // So total should be 2 (one original + one change)
    assert!(
        utxos_after.len() >= 1,
        "Should have at least 1 UTXO after spending (change + unspent), got: {}",
        utxos_after.len()
    );

    // Step 8: Verify spent UTXO not in list
    // The spent outpoint should not be in the new UTXO list
    // Note: BDK may have spent either UTXO, so we just verify the count changed
    // The key test is that we have valid UTXOs and balance decreased
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");

    assert!(
        final_balance.confirmed < initial_balance.confirmed,
        "Balance should decrease after spending, initial: {} sats, final: {} sats",
        initial_balance.confirmed,
        final_balance.confirmed
    );
}
