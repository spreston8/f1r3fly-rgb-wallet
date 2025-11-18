//! Module 1: Wallet Creation & Persistence Tests
//!
//! Tests BitcoinWallet initialization, SQLite persistence, and wallet isolation.

use crate::common::TestBitcoinEnv;
use bdk_wallet::KeychainKind;
use f1r3fly_rgb_wallet::bitcoin::BitcoinWallet;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;
use f1r3fly_rgb_wallet::storage::models::WalletKeys;
use std::fs;

/// Test 1.1: Verify BitcoinWallet initializes with SQLite and persists state
#[tokio::test]
async fn test_bitcoin_wallet_initialization_with_sqlite_persistence() {
    let env = TestBitcoinEnv::new("wallet_sqlite_persistence");

    // Step 1: Create wallet keys
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_name = env.unique_wallet_name();
    let wallet_dir = env.wallet_dir(wallet_name);

    // Step 2: Initialize BitcoinWallet with descriptor and wallet directory
    fs::create_dir_all(&wallet_dir).expect("Failed to create wallet directory");

    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 3: Verify SQLite database file created
    let db_path = wallet_dir.join("bitcoin.db");
    assert!(
        db_path.exists(),
        "SQLite database should be created at {:?}",
        db_path
    );

    // Step 4: Reveal first address (this marks it as used and increments index)
    let first_reveal = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let first_address = first_reveal.address.to_string();

    // Verify address is valid regtest address
    assert!(
        first_address.starts_with("bcrt1"),
        "Address should be regtest (bcrt1), got: {}",
        first_address
    );

    // Persist wallet state after revealing address
    wallet.persist().expect("Failed to persist wallet");

    // Step 5: Drop wallet instance
    drop(wallet);

    // Step 6: Create new BitcoinWallet instance pointing to same DB
    let mut wallet2 = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to reload BitcoinWallet from DB");

    // Step 7: Verify wallet loads existing state (address index preserved)
    // The next revealed address should be different (index incremented)
    let second_reveal = wallet2
        .inner_mut()
        .reveal_next_address(KeychainKind::External);
    let second_address = second_reveal.address.to_string();

    // Step 8: Verify addresses are different
    // Since we persisted after revealing first address, next reveal should give second address
    assert_ne!(
        first_address, second_address,
        "Second address should be different from first"
    );

    // Verify both are valid regtest addresses
    assert!(second_address.starts_with("bcrt1"));

    // Verify we can peek at both addresses by index
    let peeked_first = wallet2.inner().peek_address(KeychainKind::External, 0);
    let peeked_second = wallet2.inner().peek_address(KeychainKind::External, 1);

    assert_eq!(
        first_address,
        peeked_first.address.to_string(),
        "First address should be retrievable from persisted state"
    );

    assert_eq!(
        second_address,
        peeked_second.address.to_string(),
        "Second address should be retrievable from persisted state"
    );
}

/// Test 1.2: Verify wallet produces correct regtest addresses
#[tokio::test]
async fn test_wallet_network_specific_addresses() {
    let env = TestBitcoinEnv::new("wallet_network_addresses");

    // Step 1: Create wallet keys
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_name = env.unique_wallet_name();
    let wallet_dir = env.wallet_dir(wallet_name);
    fs::create_dir_all(&wallet_dir).expect("Failed to create wallet directory");

    // Step 2: Create BitcoinWallet for regtest
    let wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 3: Get 5 external (receive) addresses
    let mut external_addresses = Vec::new();
    for i in 0..5 {
        let addr_info = wallet.inner().peek_address(KeychainKind::External, i);
        external_addresses.push(addr_info.address.to_string());
    }

    // Step 4: Get 5 internal (change) addresses
    let mut internal_addresses = Vec::new();
    for i in 0..5 {
        let addr_info = wallet.inner().peek_address(KeychainKind::Internal, i);
        internal_addresses.push(addr_info.address.to_string());
    }

    // Step 5: Verify all start with `bcrt1` (regtest prefix)
    for (i, addr) in external_addresses.iter().enumerate() {
        assert!(
            addr.starts_with("bcrt1"),
            "External address {} should start with bcrt1, got: {}",
            i,
            addr
        );
    }

    for (i, addr) in internal_addresses.iter().enumerate() {
        assert!(
            addr.starts_with("bcrt1"),
            "Internal address {} should start with bcrt1, got: {}",
            i,
            addr
        );
    }

    // Step 6: Verify external and internal addresses are different
    for ext_addr in &external_addresses {
        for int_addr in &internal_addresses {
            assert_ne!(
                ext_addr, int_addr,
                "External and internal addresses should be different"
            );
        }
    }

    // Step 7: Verify addresses are valid P2WPKH (witness version 0, 20-byte hash)
    // bcrt1 addresses should be bech32 format
    for addr in external_addresses.iter().chain(internal_addresses.iter()) {
        // Parse as Bitcoin address to verify validity
        let parsed = addr
            .parse::<bdk_wallet::bitcoin::Address<bdk_wallet::bitcoin::address::NetworkUnchecked>>()
            .expect(&format!("Address should be valid: {}", addr));

        // Verify it's for regtest network
        let checked = parsed
            .require_network(bdk_wallet::bitcoin::Network::Regtest)
            .expect(&format!("Address should be valid for regtest: {}", addr));

        // Verify it's a witness address (P2WPKH)
        assert!(
            checked.is_spend_standard(),
            "Address should be standard witness address: {}",
            addr
        );
    }

    // Step 8: Verify addresses in same keychain are sequential
    for i in 1..external_addresses.len() {
        assert_ne!(
            external_addresses[i - 1],
            external_addresses[i],
            "Sequential addresses should be different"
        );
    }

    for i in 1..internal_addresses.len() {
        assert_ne!(
            internal_addresses[i - 1],
            internal_addresses[i],
            "Sequential addresses should be different"
        );
    }
}

/// Test 1.3: Verify multiple wallets don't interfere with each other
#[tokio::test]
async fn test_multiple_wallets_independent_state() {
    let env = TestBitcoinEnv::new("multiple_wallets_isolation");

    // Step 1: Create wallet1
    let mnemonic1 = generate_mnemonic().expect("Failed to generate mnemonic1");
    let keys1 = WalletKeys::from_mnemonic(&mnemonic1, NetworkType::Regtest)
        .expect("Failed to derive keys1");

    let wallet1_name = format!("{}_wallet1", env.unique_wallet_name());
    let wallet1_dir = env.wallet_dir(&wallet1_name);
    fs::create_dir_all(&wallet1_dir).expect("Failed to create wallet1 directory");

    let mut wallet1 = BitcoinWallet::new(
        keys1.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet1_dir,
    )
    .expect("Failed to create wallet1");

    // Step 2: Create wallet2 with different mnemonic
    let mnemonic2 = generate_mnemonic().expect("Failed to generate mnemonic2");
    let keys2 = WalletKeys::from_mnemonic(&mnemonic2, NetworkType::Regtest)
        .expect("Failed to derive keys2");

    let wallet2_name = format!("{}_wallet2", env.unique_wallet_name());
    let wallet2_dir = env.wallet_dir(&wallet2_name);
    fs::create_dir_all(&wallet2_dir).expect("Failed to create wallet2 directory");

    let mut wallet2 = BitcoinWallet::new(
        keys2.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet2_dir,
    )
    .expect("Failed to create wallet2");

    // Step 3: Get addresses from both wallets (reveal to mark as used)
    let wallet1_addr = wallet1
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let wallet2_addr = wallet2
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    // Step 4: Verify different DB files created
    let db1_path = wallet1_dir.join("bitcoin.db");
    let db2_path = wallet2_dir.join("bitcoin.db");

    assert!(db1_path.exists(), "Wallet1 DB should exist");
    assert!(db2_path.exists(), "Wallet2 DB should exist");
    assert_ne!(db1_path, db2_path, "DB paths should be different");

    // Step 5: Verify addresses are different (different mnemonics = different keys)
    assert_ne!(
        wallet1_addr, wallet2_addr,
        "Wallets with different mnemonics should have different addresses"
    );

    // Step 6: Fund wallet1 only
    let txid = env
        .fund_address(&wallet1_addr, 0.1)
        .expect("Failed to fund wallet1");

    // Mine 1 block to confirm
    env.mine_blocks(1).expect("Failed to mine block");

    // Wait for confirmation
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // Step 7: Sync both wallets
    use f1r3fly_rgb_wallet::bitcoin::{sync_wallet, EsploraClient};

    let esplora_client =
        EsploraClient::new(&env.config().bitcoin.esplora_url, NetworkType::Regtest)
            .expect("Failed to create Esplora client");

    sync_wallet(&mut wallet1, &esplora_client).expect("Failed to sync wallet1");
    sync_wallet(&mut wallet2, &esplora_client).expect("Failed to sync wallet2");

    // Step 8: Verify only wallet1 has balance
    use f1r3fly_rgb_wallet::bitcoin::get_balance;

    let balance1 = get_balance(&wallet1).expect("Failed to get wallet1 balance");
    let balance2 = get_balance(&wallet2).expect("Failed to get wallet2 balance");

    assert!(
        balance1.confirmed > 0,
        "Wallet1 should have confirmed balance, got: {} sats",
        balance1.confirmed
    );

    assert_eq!(
        balance2.confirmed, 0,
        "Wallet2 should have zero balance, got: {} sats",
        balance2.confirmed
    );

    assert_eq!(
        balance2.unconfirmed, 0,
        "Wallet2 should have zero unconfirmed balance, got: {} sats",
        balance2.unconfirmed
    );

    // Step 9: Verify balances are independent
    // Reveal another address from wallet2 and verify it's different
    let wallet2_addr2 = wallet2
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    assert_ne!(
        wallet2_addr, wallet2_addr2,
        "Wallet2 should generate new addresses (first was already revealed)"
    );

    // Sync wallet2 again and verify still empty
    sync_wallet(&mut wallet2, &esplora_client).expect("Failed to sync wallet2 again");
    let balance2_after = get_balance(&wallet2).expect("Failed to get wallet2 balance after");

    assert_eq!(
        balance2_after.confirmed, 0,
        "Wallet2 should still have zero balance after second sync"
    );

    // Verify wallet1 balance unchanged
    let balance1_after = get_balance(&wallet1).expect("Failed to get wallet1 balance after");
    assert_eq!(
        balance1_after.confirmed, balance1.confirmed,
        "Wallet1 balance should remain unchanged"
    );
}
