//! Module 2: Network & Sync Operations Tests
//!
//! Tests for Esplora client connectivity and wallet synchronization functionality.
//! Covers empty wallet sync, fund detection, idempotent sync, and sync after spending.

use bdk_wallet::KeychainKind;
use f1r3fly_rgb_wallet::bitcoin::balance::get_balance;
use f1r3fly_rgb_wallet::bitcoin::network::EsploraClient;
use f1r3fly_rgb_wallet::bitcoin::sync::sync_wallet;
use f1r3fly_rgb_wallet::bitcoin::wallet::BitcoinWallet;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;
use f1r3fly_rgb_wallet::storage::models::WalletKeys;

use crate::common::TestBitcoinEnv;

/// Test 2.1: Verify EsploraClient connects to regtest Esplora and queries height
#[tokio::test]
async fn test_esplora_client_connection_and_height_query() {
    // Step 1: Create test environment
    let env = TestBitcoinEnv::new("esplora_connection");

    // Step 2: Create EsploraClient pointing to regtest
    let esplora_url = "http://localhost:3002";
    let client = EsploraClient::new(esplora_url, NetworkType::Regtest)
        .expect("Failed to create EsploraClient");

    // Step 3: Call is_available() - verify Esplora is reachable
    let is_available = client
        .is_available()
        .expect("Failed to check Esplora availability");
    assert!(
        is_available,
        "Esplora should be available at {}",
        esplora_url
    );

    // Step 4: Call get_height()
    let esplora_height = client
        .get_height()
        .expect("Failed to get height from Esplora");

    // Step 5: Call get_tip_hash()
    let tip_hash = client
        .get_tip_hash()
        .expect("Failed to get tip hash from Esplora");

    // Step 6: Use bitcoin-cli to get blockchain height
    let rpc_height = env
        .bitcoin_rpc()
        .get_block_count()
        .expect("Failed to get block count from bitcoin-cli");

    // Step 7: Compare heights (should match or be within 1 block due to indexing delay)
    let height_diff = if esplora_height > rpc_height {
        esplora_height - rpc_height
    } else {
        rpc_height - esplora_height
    };

    assert!(
        height_diff <= 1,
        "Esplora height ({}) and RPC height ({}) differ by more than 1 block",
        esplora_height,
        rpc_height
    );

    // Step 8: Verify tip hash is valid (64 hex characters when formatted as string)
    let tip_hash_str = tip_hash.to_string();
    assert_eq!(
        tip_hash_str.len(),
        64,
        "Tip hash should be 64 hex characters, got: {}",
        tip_hash_str
    );

    // Verify tip hash is valid hex
    assert!(
        tip_hash_str.chars().all(|c| c.is_ascii_hexdigit()),
        "Tip hash should be valid hex: {}",
        tip_hash_str
    );
}

/// Test 2.2: Verify syncing empty wallet works
#[tokio::test]
async fn test_wallet_sync_with_empty_wallet() {
    // Step 1: Create test environment with new wallet
    let env = TestBitcoinEnv::new("empty_wallet_sync");

    // Step 2: Generate wallet keys
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("empty_wallet");

    // Step 3: Create BitcoinWallet
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 4: Get current blockchain height for reference
    let blockchain_height = env
        .get_blockchain_height()
        .expect("Failed to get blockchain height");

    // Step 5: Call sync_wallet()
    let sync_result =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync empty wallet");

    // Step 6: Verify SyncResult returned
    assert!(
        sync_result.height >= blockchain_height,
        "Sync height ({}) should be at least blockchain height ({})",
        sync_result.height,
        blockchain_height
    );

    // Step 7: Verify new_txs = 0
    assert_eq!(
        sync_result.new_txs, 0,
        "Empty wallet should have 0 new transactions, got: {}",
        sync_result.new_txs
    );

    // Step 8: Verify balance = 0
    let balance = get_balance(&wallet).expect("Failed to get balance");
    assert_eq!(
        balance.confirmed, 0,
        "Empty wallet should have 0 confirmed balance, got: {} sats",
        balance.confirmed
    );
    assert_eq!(
        balance.unconfirmed, 0,
        "Empty wallet should have 0 unconfirmed balance, got: {} sats",
        balance.unconfirmed
    );
    assert_eq!(
        balance.total, 0,
        "Empty wallet should have 0 total balance, got: {} sats",
        balance.total
    );
}

/// Test 2.3: Verify sync detects incoming transactions
#[tokio::test]
async fn test_wallet_sync_detects_received_funds() {
    // Step 1: Create test environment with new wallet
    let env = TestBitcoinEnv::new("sync_detects_funds");

    // Step 2: Generate wallet keys and create wallet
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("funded_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Step 3: Get wallet address
    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    // Step 4: Record initial sync state
    let initial_sync =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to perform initial sync");
    let initial_height = initial_sync.height;
    let initial_tx_count = wallet.inner().transactions().count();

    // Step 5: Send 0.1 BTC to wallet address from regtest
    let amount_btc = 0.1;
    let txid = env
        .fund_address(&address, amount_btc)
        .expect("Failed to fund address");

    // Step 6: Mine 1 block and wait for confirmation
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm transaction");

    // Step 7: Sync wallet again
    let after_sync =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after funding");

    // Step 8: Verify SyncResult shows new_txs > 0
    assert!(
        after_sync.new_txs > 0,
        "Should detect new transactions, got: {}",
        after_sync.new_txs
    );

    // Step 9: Verify height increased by at least 1
    assert!(
        after_sync.height > initial_height,
        "Height should increase from {} to at least {}",
        initial_height,
        initial_height + 1
    );

    // Step 10: Verify balance > 0
    let balance = get_balance(&wallet).expect("Failed to get balance");
    let expected_sats = (amount_btc * 100_000_000.0) as u64;

    assert!(
        balance.confirmed > 0,
        "Wallet should have confirmed balance after sync, got: {} sats",
        balance.confirmed
    );

    assert_eq!(
        balance.confirmed, expected_sats,
        "Balance should be {} sats (0.1 BTC), got: {} sats",
        expected_sats, balance.confirmed
    );

    // Step 11: Verify transaction count increased
    let final_tx_count = wallet.inner().transactions().count();
    assert!(
        final_tx_count > initial_tx_count,
        "Transaction count should increase from {} to at least {}",
        initial_tx_count,
        initial_tx_count + 1
    );
}

/// Test 2.4: Verify repeated syncs don't duplicate data
#[tokio::test]
async fn test_wallet_sync_idempotent() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("sync_idempotent");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("idempotent_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Get address and fund it
    let address = wallet
        .inner_mut()
        .reveal_next_address(KeychainKind::External)
        .address
        .to_string();

    let txid = env
        .fund_address(&address, 0.5)
        .expect("Failed to fund address");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 2: Sync wallet
    let first_sync =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to perform first sync");

    // Step 3: Record state after first sync
    let first_height = first_sync.height;
    let first_tx_count = wallet.inner().transactions().count();
    let first_balance = get_balance(&wallet).expect("Failed to get balance after first sync");

    // Verify we got the funds
    assert!(
        first_balance.confirmed > 0,
        "First sync should detect funds"
    );

    // Step 4: Sync wallet again immediately (no new blocks)
    let second_sync =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to perform second sync");

    // Step 5: Verify SyncResult unchanged
    assert_eq!(
        second_sync.height, first_height,
        "Height should remain {} on second sync, got: {}",
        first_height, second_sync.height
    );

    assert_eq!(
        second_sync.new_txs, 0,
        "Second sync should have 0 new transactions (no new blocks), got: {}",
        second_sync.new_txs
    );

    // Step 6: Verify transaction count unchanged
    let second_tx_count = wallet.inner().transactions().count();
    assert_eq!(
        second_tx_count, first_tx_count,
        "Transaction count should remain {}, got: {}",
        first_tx_count, second_tx_count
    );

    // Step 7: Verify balance unchanged
    let second_balance = get_balance(&wallet).expect("Failed to get balance after second sync");
    assert_eq!(
        second_balance.confirmed, first_balance.confirmed,
        "Confirmed balance should remain {} sats, got: {} sats",
        first_balance.confirmed, second_balance.confirmed
    );
    assert_eq!(
        second_balance.total, first_balance.total,
        "Total balance should remain {} sats, got: {} sats",
        first_balance.total, second_balance.total
    );
}

/// Test 2.5: Verify sync detects spent UTXOs
#[tokio::test]
async fn test_sync_updates_wallet_after_spending() {
    // Step 1: Create funded wallet
    let env = TestBitcoinEnv::new("sync_after_spending");

    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    let wallet_dir = env.wallet_dir("spending_wallet");
    let mut wallet = BitcoinWallet::new(
        keys.bitcoin_descriptor.clone(),
        NetworkType::Regtest,
        &wallet_dir,
    )
    .expect("Failed to create BitcoinWallet");

    // Fund wallet with 1 BTC
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

    // Step 2: Sync and record initial state
    sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after funding");

    let initial_balance = get_balance(&wallet).expect("Failed to get initial balance");
    let initial_utxo_count = wallet.inner().list_unspent().count();

    assert!(
        initial_balance.confirmed > 0,
        "Wallet should have funds, got: {} sats",
        initial_balance.confirmed
    );
    assert!(
        initial_utxo_count > 0,
        "Wallet should have UTXOs, got: {}",
        initial_utxo_count
    );

    // Step 3: Send Bitcoin to external address
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    let send_amount = 50_000_000; // 0.5 BTC in sats

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

    // Step 4: Mine 1 block to confirm spend
    env.wait_for_confirmation(&spend_txid.to_string(), 1)
        .await
        .expect("Failed to confirm spend transaction");

    // Step 5: Sync wallet
    let after_spend_sync =
        sync_wallet(&mut wallet, &env.esplora_client).expect("Failed to sync after spending");

    assert!(
        after_spend_sync.new_txs > 0,
        "Sync should detect spending transaction"
    );

    // Step 6: Verify UTXO count reflects spend (may have change UTXO)
    let final_utxo_count = wallet.inner().list_unspent().count();

    // Original UTXO should be spent; might have change UTXO
    // So final count should be 0 (if no change) or 1 (if change created)
    assert!(
        final_utxo_count <= initial_utxo_count,
        "UTXO count should not increase after spending, initial: {}, final: {}",
        initial_utxo_count,
        final_utxo_count
    );

    // Step 7: Verify balance decreased by amount + fee
    let final_balance = get_balance(&wallet).expect("Failed to get final balance");

    assert!(
        final_balance.confirmed < initial_balance.confirmed,
        "Balance should decrease after spending, initial: {} sats, final: {} sats",
        initial_balance.confirmed,
        final_balance.confirmed
    );

    let balance_decrease = initial_balance.confirmed - final_balance.confirmed;
    assert!(
        balance_decrease >= send_amount,
        "Balance decrease ({} sats) should be at least send amount ({} sats)",
        balance_decrease,
        send_amount
    );

    // Verify the decrease is reasonable (send amount + fee, fee should be < 10,000 sats for regtest)
    let max_expected_decrease = send_amount + 10_000;
    assert!(
        balance_decrease <= max_expected_decrease,
        "Balance decrease ({} sats) exceeds send amount + max fee ({} sats)",
        balance_decrease,
        max_expected_decrease
    );
}
