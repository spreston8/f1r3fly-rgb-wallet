//! Wallet State Persistence & Isolation Tests
//!
//! Tests for F1r3fly RGB state persistence across wallet reloads and isolation
//! between multiple wallets.

use super::{check_f1r3node_available, setup_wallet_with_genesis_utxo};
use crate::common::TestBitcoinEnv;
use f1r3fly_rgb_wallet::manager::WalletManager;

/// Test RGB state persists across wallet reload
///
/// Critical test ensuring users don't lose RGB assets when closing/reopening wallet.
///
/// Verifies:
/// - Contract metadata persists across WalletManager drop/reload
/// - Balances remain correct after reload
/// - Can issue new assets after reload
/// - Multiple reload cycles work correctly
/// Known issue: Test fails non-deterministically (1 in 3-5 runs). See docs/bugs/non-deterministic-wallet-reload-failure.md
#[ignore]
#[tokio::test]
async fn test_state_persists_across_wallet_reload() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("state_persists_reload");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Step 1: Create wallet and issue first asset
    let asset_1_contract_id = {
        let (mut manager, genesis_utxo) =
            setup_wallet_with_genesis_utxo(&env, wallet_name, password)
                .await
                .expect("Failed to setup wallet");

        let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
            ticker: "USD".to_string(),
            name: "US Dollar".to_string(),
            supply: 1_000_000,
            precision: 2,
            genesis_utxo,
        };

        let asset_info = manager
            .issue_asset(request)
            .await
            .expect("Failed to issue USD");

        asset_info.contract_id.clone()
    }; // manager dropped here

    // Step 2: Reload wallet and verify first asset persists
    {
        let config = env.config().clone();
        let mut manager = WalletManager::new(config).expect("Failed to create manager");
        manager
            .load_wallet(wallet_name, password)
            .expect("Failed to load wallet");

        // Sync wallet to restore Bitcoin state
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync after reload");

        // Verify list_assets shows USD
        let assets = manager.list_assets().expect("Failed to list assets");
        assert_eq!(assets.len(), 1, "Should have 1 asset after reload");
        assert_eq!(assets[0].ticker, "USD");
        assert_eq!(assets[0].contract_id, asset_1_contract_id);

        // Verify get_asset_info returns correct metadata
        let asset_info = manager
            .get_asset_info(&asset_1_contract_id)
            .expect("Failed to get asset info");
        assert_eq!(asset_info.ticker, "USD");
        assert_eq!(asset_info.name, "US Dollar");
        assert_eq!(asset_info.supply, 1_000_000);
        assert_eq!(asset_info.precision, 2);

        // Verify balance is correct
        let balance = manager
            .get_asset_balance(&asset_1_contract_id)
            .await
            .expect("Failed to get balance");
        assert_eq!(balance.total, 1_000_000, "Balance should persist");
        assert_eq!(balance.ticker, "USD");
    } // manager dropped again

    // Step 3: Reload and issue second asset
    let asset_2_contract_id = {
        let config = env.config().clone();
        let mut manager =
            WalletManager::new(config).expect("Failed to create manager for second asset");
        manager
            .load_wallet(wallet_name, password)
            .expect("Failed to load wallet for second asset");

        // Sync wallet
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync after reload");

        // Create second genesis UTXO
        let amount_sats = 1_000_000u64;
        let fee_config = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
        let genesis_result = manager
            .create_utxo(amount_sats, &fee_config, false)
            .expect("Failed to create second UTXO");
        let genesis_txid = genesis_result.txid.clone();

        env.wait_for_confirmation(&genesis_txid, 1)
            .await
            .expect("Failed to confirm second UTXO");

        // Sync with retry for second UTXO
        for attempt in 1..=5 {
            manager.sync_wallet().await.expect("Failed to sync");
            let utxos: Vec<_> = manager
                .bitcoin_wallet()
                .expect("Bitcoin wallet not loaded")
                .inner()
                .list_unspent()
                .collect();
            if utxos
                .iter()
                .any(|u| u.outpoint.txid.to_string() == genesis_txid)
            {
                break;
            }
            if attempt < 5 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        let genesis_utxo_2 = format!("{}:{}", genesis_result.txid, genesis_result.outpoint.vout);

        let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
            ticker: "EUR".to_string(),
            name: "Euro".to_string(),
            supply: 2_000_000,
            precision: 2,
            genesis_utxo: genesis_utxo_2,
        };

        let asset_info = manager
            .issue_asset(request)
            .await
            .expect("Failed to issue EUR after reload");

        asset_info.contract_id.clone()
    }; // manager dropped

    // Step 4: Final reload - verify both assets tracked correctly
    {
        let config = env.config().clone();
        let mut manager =
            WalletManager::new(config).expect("Failed to create manager for final check");
        manager
            .load_wallet(wallet_name, password)
            .expect("Failed to load wallet for final check");

        // Sync wallet
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync after reload");

        // Verify both assets in list_assets
        let assets = manager.list_assets().expect("Failed to list assets");
        assert_eq!(assets.len(), 2, "Should have 2 assets after reload");

        let usd_asset = assets.iter().find(|a| a.ticker == "USD");
        let eur_asset = assets.iter().find(|a| a.ticker == "EUR");
        assert!(usd_asset.is_some(), "USD should be present");
        assert!(eur_asset.is_some(), "EUR should be present");

        // Verify both balances
        let balance_usd = manager
            .get_asset_balance(&asset_1_contract_id)
            .await
            .expect("Failed to get USD balance");
        assert_eq!(balance_usd.total, 1_000_000);

        let balance_eur = manager
            .get_asset_balance(&asset_2_contract_id)
            .await
            .expect("Failed to get EUR balance");
        assert_eq!(balance_eur.total, 2_000_000);

        // Verify state file contains both contracts
        let wallet_dir = env.wallet_dir(wallet_name);
        let state_file = wallet_dir.join("f1r3fly_state.json");
        let state_content =
            std::fs::read_to_string(&state_file).expect("Failed to read state file");
        let state: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
            serde_json::from_str(&state_content).expect("Failed to parse state");

        assert_eq!(state.contracts_metadata.len(), 2);
        assert_eq!(state.genesis_utxos.len(), 2);
    }
}

/// Test multiple wallets have isolated RGB state
///
/// Verifies:
/// - RGB assets in wallet A don't appear in wallet B
/// - State files are separate per wallet
/// - Reload preserves isolation
#[tokio::test]
async fn test_multiple_wallets_isolated_rgb_state() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("wallets_isolated");
    let wallet_a_name = format!("{}_wallet_a", env.unique_wallet_name());
    let wallet_b_name = format!("{}_wallet_b", env.unique_wallet_name());
    let password = "test_password";

    // Step 1: Create wallet A and issue USD
    let asset_a_contract_id = {
        let (mut manager, genesis_utxo) =
            setup_wallet_with_genesis_utxo(&env, &wallet_a_name, password)
                .await
                .expect("Failed to setup wallet A");

        let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
            ticker: "USD".to_string(),
            name: "US Dollar".to_string(),
            supply: 1_000_000,
            precision: 2,
            genesis_utxo,
        };

        let asset_info = manager
            .issue_asset(request)
            .await
            .expect("Failed to issue USD in wallet A");

        asset_info.contract_id.clone()
    };

    // Step 2: Create wallet B and issue EUR
    let asset_b_contract_id = {
        let (mut manager, genesis_utxo) =
            setup_wallet_with_genesis_utxo(&env, &wallet_b_name, password)
                .await
                .expect("Failed to setup wallet B");

        let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
            ticker: "EUR".to_string(),
            name: "Euro".to_string(),
            supply: 2_000_000,
            precision: 2,
            genesis_utxo,
        };

        let asset_info = manager
            .issue_asset(request)
            .await
            .expect("Failed to issue EUR in wallet B");

        asset_info.contract_id.clone()
    };

    // Step 3: Verify wallet A only has USD
    {
        let config = env.config().clone();
        let mut manager = WalletManager::new(config).expect("Failed to create manager");
        manager
            .load_wallet(&wallet_a_name, password)
            .expect("Failed to load wallet A");

        // Sync wallet
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync wallet A");

        let assets = manager.list_assets().expect("Failed to list assets");
        assert_eq!(assets.len(), 1, "Wallet A should have 1 asset");
        assert_eq!(assets[0].ticker, "USD");
        assert_eq!(assets[0].contract_id, asset_a_contract_id);

        // Verify cannot query EUR balance
        let result = manager.get_asset_balance(&asset_b_contract_id).await;
        assert!(result.is_err(), "Wallet A should not have EUR asset");
    }

    // Step 4: Verify wallet B only has EUR
    {
        let config = env.config().clone();
        let mut manager = WalletManager::new(config).expect("Failed to create manager");
        manager
            .load_wallet(&wallet_b_name, password)
            .expect("Failed to load wallet B");

        // Sync wallet
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync wallet B");

        let assets = manager.list_assets().expect("Failed to list assets");
        assert_eq!(assets.len(), 1, "Wallet B should have 1 asset");
        assert_eq!(assets[0].ticker, "EUR");
        assert_eq!(assets[0].contract_id, asset_b_contract_id);

        // Verify cannot query USD balance
        let result = manager.get_asset_balance(&asset_a_contract_id).await;
        assert!(result.is_err(), "Wallet B should not have USD asset");
    }

    // Step 5: Verify separate state files exist
    let state_a_path = env.wallet_dir(&wallet_a_name).join("f1r3fly_state.json");
    let state_b_path = env.wallet_dir(&wallet_b_name).join("f1r3fly_state.json");

    assert!(state_a_path.exists(), "Wallet A state file should exist");
    assert!(state_b_path.exists(), "Wallet B state file should exist");

    let state_a: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&std::fs::read_to_string(&state_a_path).unwrap()).unwrap();
    let state_b: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&std::fs::read_to_string(&state_b_path).unwrap()).unwrap();

    assert_eq!(state_a.contracts_metadata.len(), 1);
    assert_eq!(state_b.contracts_metadata.len(), 1);
    assert!(state_a
        .contracts_metadata
        .contains_key(&asset_a_contract_id));
    assert!(state_b
        .contracts_metadata
        .contains_key(&asset_b_contract_id));
}

/// Test genesis UTXO tracking API
///
/// Verifies:
/// - genesis_utxos() returns correct data
/// - Multiple genesis UTXOs tracked correctly
/// - GenesisUtxoInfo has all expected fields
#[tokio::test]
async fn test_genesis_utxo_tracking() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("genesis_utxo_tracking");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet and issue first asset
    let (mut manager, genesis_utxo_1) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    let request_1 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "USD".to_string(),
        name: "US Dollar".to_string(),
        supply: 1_000_000,
        precision: 2,
        genesis_utxo: genesis_utxo_1.clone(),
    };

    let asset_1 = manager
        .issue_asset(request_1)
        .await
        .expect("Failed to issue first asset");

    // Create second genesis UTXO
    let amount_sats = 1_000_000u64;
    let fee_config = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
    let genesis_result_2 = manager
        .create_utxo(amount_sats, &fee_config, false)
        .expect("Failed to create second UTXO");
    let genesis_txid_2 = genesis_result_2.txid.clone();

    env.wait_for_confirmation(&genesis_txid_2, 1)
        .await
        .expect("Failed to confirm second UTXO");

    // Sync with retry
    for attempt in 1..=5 {
        manager.sync_wallet().await.expect("Failed to sync");
        let utxos: Vec<_> = manager
            .bitcoin_wallet()
            .expect("Bitcoin wallet not loaded")
            .inner()
            .list_unspent()
            .collect();
        if utxos
            .iter()
            .any(|u| u.outpoint.txid.to_string() == genesis_txid_2)
        {
            break;
        }
        if attempt < 5 {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    let genesis_utxo_2 = format!(
        "{}:{}",
        genesis_result_2.txid, genesis_result_2.outpoint.vout
    );

    let request_2 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "ETH".to_string(),
        name: "Ethereum".to_string(),
        supply: 120_000_000,
        precision: 18,
        genesis_utxo: genesis_utxo_2.clone(),
    };

    let asset_2 = manager
        .issue_asset(request_2)
        .await
        .expect("Failed to issue second asset");

    // Verify genesis UTXO tracking via state file
    // (Direct API access to contracts_manager is internal implementation)
    let wallet_dir = env.wallet_dir(wallet_name);
    let state_file = wallet_dir.join("f1r3fly_state.json");
    let state_content = std::fs::read_to_string(&state_file).expect("Failed to read state file");
    let state: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&state_content).expect("Failed to parse state");

    // Verify 2 genesis UTXOs tracked
    assert_eq!(state.genesis_utxos.len(), 2, "Should have 2 genesis UTXOs");

    // Verify genesis UTXO 1 (USD)
    let genesis_info_1 = state
        .genesis_utxos
        .get(&asset_1.contract_id)
        .expect("Should have genesis info for asset 1");

    assert_eq!(genesis_info_1.contract_id, asset_1.contract_id);
    assert_eq!(genesis_info_1.ticker, "USD");
    assert_eq!(genesis_info_1.name, "US Dollar");
    assert_eq!(genesis_info_1.supply, 1_000_000);
    assert_eq!(genesis_info_1.precision, 2);
    assert!(!genesis_info_1.txid.is_empty());
    assert!(genesis_info_1.vout < 100);

    // Verify genesis UTXO 2 (ETH)
    let genesis_info_2 = state
        .genesis_utxos
        .get(&asset_2.contract_id)
        .expect("Should have genesis info for asset 2");

    assert_eq!(genesis_info_2.contract_id, asset_2.contract_id);
    assert_eq!(genesis_info_2.ticker, "ETH");
    assert_eq!(genesis_info_2.name, "Ethereum");
    assert_eq!(genesis_info_2.supply, 120_000_000);
    assert_eq!(genesis_info_2.precision, 18);
    assert!(!genesis_info_2.txid.is_empty());
    assert!(genesis_info_2.vout < 100);

    // Verify vout matches original UTXOs
    let parts_1: Vec<&str> = genesis_utxo_1.split(':').collect();
    let parts_2: Vec<&str> = genesis_utxo_2.split(':').collect();

    assert_eq!(genesis_info_1.vout.to_string(), parts_1[1]);
    assert_eq!(genesis_info_2.vout.to_string(), parts_2[1]);
}
