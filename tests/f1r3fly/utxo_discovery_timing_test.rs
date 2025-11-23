use f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig;
/// Test to reproduce and investigate UTXO discovery timing issues
///
/// This test demonstrates the flakiness caused by Esplora's inconsistent
/// transaction indexing in regtest mode when using BDK-to-BDK transactions.
///
/// **CRITICAL**: Uses Alice's BDK wallet to send to Bob's BDK wallet (NOT Bitcoin Core RPC),
/// which properly reproduces the timing issues seen in RGB transfer tests.
use f1r3fly_rgb_wallet::types::UtxoFilter;
use std::time::Instant;

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{check_f1r3node_available, setup_test_wallets};

/// Test that reproduces the UTXO discovery timing issue
///
/// Expected behavior: After broadcasting a BDK transaction and mining a block,
/// Bob's wallet should consistently discover the new UTXO within a reasonable timeframe.
///
/// Actual behavior: Sometimes BDK discovers the UTXO immediately, sometimes it takes
/// 20-60+ seconds, and sometimes it never discovers it within the timeout period.
///
/// **IMPORTANT**: This test uses BDK→BDK transactions (Alice sends to Bob via BDK),
/// NOT Bitcoin Core RPC `send_to_address`, to properly reproduce the RGB test timing issue.
#[tokio::test]
async fn test_utxo_discovery_timing() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("utxo_discovery_timing");

    // Setup Alice and Bob's wallets (both BDK wallets)
    let wallets = setup_test_wallets(&env)
        .await
        .expect("Failed to setup test wallets");

    let mut alice = wallets.alice;
    let mut bob = wallets.bob;

    // Initial sync for both wallets
    alice.sync_wallet().await.expect("Failed to sync Alice");
    bob.sync_wallet().await.expect("Failed to sync Bob");

    println!("\n========================================");
    println!("📊 Initial State");
    println!("========================================");

    let filter = UtxoFilter {
        available_only: false,
        rgb_only: false,
        confirmed_only: false,
        min_amount_sats: None,
    };

    let bob_initial_utxos = bob
        .list_utxos(filter.clone())
        .await
        .expect("Failed to list Bob's UTXOs");
    let initial_count = bob_initial_utxos.len();
    println!("Bob's initial UTXO count: {}", initial_count);

    let alice_initial_utxos = alice
        .list_utxos(filter.clone())
        .await
        .expect("Failed to list Alice's UTXOs");
    println!("Alice's initial UTXO count: {}", alice_initial_utxos.len());

    // Send Bitcoin to Bob using Alice's BDK wallet (NOT Bitcoin Core RPC)
    println!("\n========================================");
    println!("💸 Alice (BDK wallet) sending 50,000 sats to Bob (BDK wallet)");
    println!("========================================");

    let bob_address = bob.get_new_address().expect("Failed to get Bob's address");
    println!("Bob's address: {}", bob_address);

    // Send using BDK wallet (like RGB transfers do)
    let fee_rate = FeeRateConfig::medium_priority();
    let txid = alice
        .send_bitcoin(&bob_address.to_string(), 50_000, &fee_rate)
        .expect("Failed to send bitcoin via BDK");

    println!("Transaction broadcast via BDK: {}", txid);
    let broadcast_time = Instant::now();

    // Mine a block to confirm
    println!("\n========================================");
    println!("⛏️  Mining block to confirm transaction");
    println!("========================================");

    env.mine_blocks(1).expect("Failed to mine block");
    let mine_time = Instant::now();
    println!(
        "Block mined at +{:.2}s",
        mine_time.duration_since(broadcast_time).as_secs_f64()
    );

    println!("\n========================================");
    println!("🔍 Testing Bob's UTXO Discovery");
    println!("========================================");

    // Wait a bit for Esplora to index
    println!("Waiting 3 seconds for Esplora indexing...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Now attempt to discover the UTXO with detailed logging
    let mut discovery_times = Vec::new();
    let max_attempts = 30;
    let mut utxo_found = false;

    for attempt in 1..=max_attempts {
        let attempt_start = Instant::now();

        // Sync Bob's wallet
        bob.sync_wallet().await.expect("Failed to sync Bob");

        let sync_duration = attempt_start.elapsed();

        // Check UTXO count
        let bob_utxos = bob
            .list_utxos(filter.clone())
            .await
            .expect("Failed to list Bob's UTXOs");
        let current_count = bob_utxos.len();

        let time_since_mine = mine_time.elapsed();

        println!(
            "  Attempt {:2}/{}: count={} (initial={}), sync_time={:.2}s, total_time={:.2}s",
            attempt,
            max_attempts,
            current_count,
            initial_count,
            sync_duration.as_secs_f64(),
            time_since_mine.as_secs_f64()
        );

        discovery_times.push((attempt, time_since_mine, current_count > initial_count));

        if current_count > initial_count {
            utxo_found = true;
            println!(
                "\n✅ UTXO DISCOVERED after {} attempts ({:.2}s)",
                attempt,
                time_since_mine.as_secs_f64()
            );
            break;
        }

        // Wait 2 seconds before next attempt
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    println!("\n========================================");
    println!("📈 Discovery Analysis");
    println!("========================================");

    if utxo_found {
        let (attempt, duration, _) = discovery_times.iter().find(|(_, _, found)| *found).unwrap();
        println!("Status: ✅ SUCCESS");
        println!(
            "Discovery time: {:.2}s after mining",
            duration.as_secs_f64()
        );
        println!("Attempts required: {}", attempt);
    } else {
        println!("Status: ❌ FAILED");
        println!(
            "UTXO was NEVER discovered after {} attempts and {:.2}s",
            max_attempts,
            mine_time.elapsed().as_secs_f64()
        );
        println!("\nThis demonstrates the Esplora indexing flakiness!");
    }

    println!("\n========================================");
    println!("🔬 Detailed Attempt Log");
    println!("========================================");
    for (attempt, duration, found) in &discovery_times {
        println!(
            "  Attempt {}: {:.2}s - {}",
            attempt,
            duration.as_secs_f64(),
            if *found { "✅ FOUND" } else { "❌ NOT FOUND" }
        );
    }

    // Final assertion
    assert!(
        utxo_found,
        "UTXO discovery failed - this test demonstrates the timing issue"
    );
}

/// Test that runs the discovery test multiple times to measure flakiness rate
#[tokio::test]
#[ignore] // This test takes a long time, run with --ignored flag
async fn test_utxo_discovery_flakiness_rate() {
    if !check_f1r3node_available() {
        return;
    }

    const NUM_RUNS: usize = 10;
    let mut results = Vec::new();

    println!("\n========================================");
    println!("🧪 Running {} iterations to measure flakiness", NUM_RUNS);
    println!("========================================\n");

    for run in 1..=NUM_RUNS {
        println!("--- Run {}/{} ---", run, NUM_RUNS);

        let env = TestBitcoinEnv::new(&format!("flakiness_test_{}", run));
        let wallets = setup_test_wallets(&env)
            .await
            .expect("Failed to setup wallets");
        let mut alice = wallets.alice;
        let mut bob = wallets.bob;

        alice.sync_wallet().await.expect("Failed to sync Alice");
        bob.sync_wallet().await.expect("Failed to sync Bob");

        let filter = UtxoFilter {
            available_only: false,
            rgb_only: false,
            confirmed_only: false,
            min_amount_sats: None,
        };

        let initial_count = bob
            .list_utxos(filter.clone())
            .await
            .expect("Failed to list")
            .len();

        // Send using BDK (Alice → Bob) and mine
        let bob_address = bob.get_new_address().expect("Failed to get address");

        // Use BDK wallet send (not Bitcoin Core RPC)
        let fee_rate = FeeRateConfig::medium_priority();
        alice
            .send_bitcoin(&bob_address.to_string(), 50_000, &fee_rate)
            .expect("Failed to send bitcoin via BDK");

        env.mine_blocks(1).expect("Failed to mine");

        let start_time = Instant::now();
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Try to discover with timeout
        let mut discovered = false;
        let mut attempts = 0;

        for attempt in 1..=30 {
            attempts = attempt;
            bob.sync_wallet().await.expect("Failed to sync Bob");
            let count = bob
                .list_utxos(filter.clone())
                .await
                .expect("Failed to list")
                .len();

            if count > initial_count {
                discovered = true;
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        let discovery_time = if discovered {
            Some(start_time.elapsed().as_secs_f64())
        } else {
            None
        };

        results.push((run, discovered, discovery_time, attempts));

        let status = if discovered {
            format!(
                "✅ PASS ({}s, {} attempts)",
                discovery_time.unwrap(),
                attempts
            )
        } else {
            format!("❌ FAIL (timeout after {} attempts)", attempts)
        };
        println!("  Result: {}\n", status);
    }

    // Print summary
    println!("\n========================================");
    println!("📊 Flakiness Analysis Summary");
    println!("========================================");

    let successes = results
        .iter()
        .filter(|(_, discovered, _, _)| *discovered)
        .count();
    let failures = NUM_RUNS - successes;
    let success_rate = (successes as f64 / NUM_RUNS as f64) * 100.0;

    println!("Total runs: {}", NUM_RUNS);
    println!("Successes: {} ({:.1}%)", successes, success_rate);
    println!("Failures: {} ({:.1}%)", failures, 100.0 - success_rate);

    if successes > 0 {
        let discovery_times: Vec<f64> =
            results.iter().filter_map(|(_, _, time, _)| *time).collect();

        let avg_time = discovery_times.iter().sum::<f64>() / discovery_times.len() as f64;
        let min_time = discovery_times
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        let max_time = discovery_times
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);

        println!("\nDiscovery time statistics (successful runs only):");
        println!("  Average: {:.2}s", avg_time);
        println!("  Min: {:.2}s", min_time);
        println!("  Max: {:.2}s", max_time);
    }

    println!("\n========================================");
    println!("📋 Detailed Results");
    println!("========================================");
    for (run, discovered, time, attempts) in &results {
        let status_str = if *discovered {
            format!("✅ PASS - {:.2}s ({} attempts)", time.unwrap(), attempts)
        } else {
            format!("❌ FAIL - timeout ({} attempts)", attempts)
        };
        println!("Run {:2}: {}", run, status_str);
    }

    println!("\n========================================");
    println!("💡 Conclusion");
    println!("========================================");

    if failures > 0 {
        println!(
            "Test is FLAKY - {} out of {} runs failed",
            failures, NUM_RUNS
        );
        println!("This confirms the Esplora indexing timing issue in regtest mode.");
    } else {
        println!("All runs passed - unable to reproduce flakiness in this session.");
        println!("Note: Flakiness may still occur under different timing conditions.");
    }

    // Don't fail the test - this is a diagnostic test
    // Just report the flakiness rate
}
