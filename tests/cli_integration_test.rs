//! CLI Integration Test
//!
//! This test runs the actual CLI binary (like test_cli.sh) but in Rust for:
//! - Faster iteration (no bash overhead)
//! - Better error messages
//! - Ability to inspect internal state between CLI calls
//! - Direct comparison with library functions

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper to run CLI commands and capture output
struct CliRunner {
    data_dir: PathBuf,
    bin_path: PathBuf,
}

impl CliRunner {
    fn new(data_dir: PathBuf) -> Self {
        // Get the path to the compiled binary
        let bin_path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("f1r3fly-rgb-wallet");

        Self { data_dir, bin_path }
    }

    /// Run a CLI command and return stdout
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new(&self.bin_path)
            .args(&["--data-dir", self.data_dir.to_str().unwrap()])
            .args(args)
            .env("RUST_LOG", "warn") // Suppress debug logs unless needed
            .output()
            .map_err(|e| format!("Failed to execute CLI: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "CLI command failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a CLI command with debug logging
    fn run_debug(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new(&self.bin_path)
            .args(&["--data-dir", self.data_dir.to_str().unwrap()])
            .args(args)
            .env("RUST_LOG", "debug")
            .output()
            .map_err(|e| format!("Failed to execute CLI: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Print debug logs to stderr for test visibility
        eprintln!("=== DEBUG OUTPUT ===");
        eprintln!("{}", stderr);
        eprintln!("=== STDOUT ===");
        eprintln!("{}", stdout);
        eprintln!("===================");

        if !output.status.success() {
            return Err(format!("CLI command failed:\n{}\n{}", stdout, stderr));
        }

        Ok(stdout)
    }
}

/// Helper to interact with regtest Bitcoin node
struct RegtestHelper {
    bitcoin_cli: String,
}

impl RegtestHelper {
    fn new() -> Self {
        // Detect environment (CI vs local)
        let bitcoin_cli = if std::env::var("CI").is_ok() {
            let compose_file = std::env::var("COMPOSE_FILE")
                .unwrap_or_else(|_| "ci/docker-compose.yml".to_string());
            format!(
                "docker compose -f {} exec -T bitcoind bitcoin-cli -regtest -rpcuser=user -rpcpassword=password",
                compose_file
            )
        } else {
            let datadir = std::env::var("BITCOIN_DATADIR").unwrap_or_else(|_| {
                let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                path.pop();
                path.push(".bitcoin");
                path.to_str().unwrap().to_string()
            });
            format!("bitcoin-cli -regtest -datadir={}", datadir)
        };

        Self { bitcoin_cli }
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut cmd_parts: Vec<&str> = self.bitcoin_cli.split_whitespace().collect();
        cmd_parts.extend(args);

        let output = Command::new(cmd_parts[0])
            .args(&cmd_parts[1..])
            .output()
            .map_err(|e| format!("Bitcoin CLI failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Bitcoin command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn mine_blocks(&self, count: u32, address: &str) -> Result<(), String> {
        self.run(&["generatetoaddress", &count.to_string(), address])?;
        Ok(())
    }

    fn send_to_address(&self, address: &str, amount: &str) -> Result<(), String> {
        self.run(&["-rpcwallet=mining_wallet", "sendtoaddress", address, amount])?;
        Ok(())
    }
}

/// Parse contract ID from issue-asset output
fn parse_contract_id(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.contains("Contract ID:"))
        .and_then(|line| line.split_whitespace().nth(2))
        .map(|s| s.to_string())
}

/// Parse balance from rgb-balance output
fn parse_rgb_balance(output: &str) -> Option<u64> {
    output
        .lines()
        .find(|line| line.contains("Total:") || line.contains("Balance:"))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|word| word.parse::<u64>().ok())
        })
}

/// Parse Bitcoin address from get-addresses output
fn parse_address(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.contains("bcrt1"))
        .and_then(|line| {
            line.split_whitespace()
                .find(|word| word.starts_with("bcrt1"))
                .map(|s| s.to_string())
        })
}

/// Parse invoice from generate-invoice output
fn parse_invoice(output: &str) -> Option<String> {
    let mut found_header = false;
    for line in output.lines() {
        if line.contains("Invoice String:") {
            found_header = true;
        } else if found_header && line.trim().starts_with("contract:") {
            return Some(line.trim().to_string());
        }
    }
    None
}

/// Parse consignment path from send-transfer output
fn parse_consignment_path(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.contains("Consignment:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .map(|s| s.to_string())
}

/// Parse pubkey from wallet get-f1r3fly-pubkey output
fn parse_pubkey(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| {
            let trimmed = line.trim();
            // Accept both compressed (66 chars) and uncompressed (130 chars) formats
            (trimmed.len() == 66 || trimmed.len() == 130)
                && trimmed.chars().all(|c| c.is_ascii_hexdigit())
        })
        .map(|s| s.trim().to_string())
}

/// Parse available UTXO from list-utxos output
fn parse_available_utxo(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.contains(':') && line.contains("available"))
        .and_then(|line| line.split_whitespace().next())
        .map(|s| s.to_string())
}

#[test]
#[ignore] // Run with: cargo test --test cli_integration_test -- --ignored --nocapture
fn test_cli_complete_transfer_flow() {
    // Check prerequisites
    let f1r3node_running = reqwest::blocking::get("http://localhost:40403/api/status").is_ok();
    let regtest_running = reqwest::blocking::get("http://localhost:3002").is_ok();

    if !f1r3node_running || !regtest_running {
        eprintln!("⚠ Skipping CLI test:");
        if !f1r3node_running {
            eprintln!("  - F1r3node not running (http://localhost:40403)");
        }
        if !regtest_running {
            eprintln!("  - Regtest not running (http://localhost:3002)");
        }
        return;
    }

    println!("✓ Prerequisites met (F1r3node + regtest running)");

    // Setup
    let temp_dir = TempDir::new().unwrap();
    let cli = CliRunner::new(temp_dir.path().to_path_buf());
    let bitcoin = RegtestHelper::new();

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let alice_wallet = format!("test_alice_{}", timestamp);
    let bob_wallet = format!("test_bob_{}", timestamp);
    let password = "testpass123";

    println!("\n=== Setup ===");
    println!("Temp dir: {:?}", temp_dir.path());
    println!("Alice wallet: {}", alice_wallet);
    println!("Bob wallet: {}", bob_wallet);

    // Step 1: Create Alice's wallet
    println!("\n=== Step 1: Create Alice's Wallet ===");
    let output = cli
        .run(&["wallet", "create", &alice_wallet, "--password", password])
        .expect("Failed to create Alice's wallet");
    assert!(
        output.contains("created successfully"),
        "Alice wallet creation failed"
    );
    println!("✓ Alice's wallet created");

    // Step 2: Get Alice's address
    println!("\n=== Step 2: Get Alice's Address ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "get-addresses",
            "--count",
            "1",
            "--password",
            password,
        ])
        .expect("Failed to get Alice's address");

    let alice_address = parse_address(&output).expect("Failed to parse Alice's address");
    println!("✓ Alice's address: {}", alice_address);

    // Step 3: Fund Alice's wallet
    println!("\n=== Step 3: Fund Alice's Wallet ===");
    bitcoin
        .send_to_address(&alice_address, "0.01")
        .expect("Failed to send BTC to Alice");
    bitcoin
        .mine_blocks(1, &alice_address)
        .expect("Failed to mine blocks");
    println!("✓ Sent 0.01 BTC to Alice and mined 1 block");

    std::thread::sleep(std::time::Duration::from_secs(3));

    // Step 4: Sync Alice's wallet
    println!("\n=== Step 4: Sync Alice's Wallet ===");
    cli.run(&["--wallet", &alice_wallet, "sync", "--password", password])
        .expect("Failed to sync Alice's wallet");
    println!("✓ Alice's wallet synced");

    // Step 5: Get available UTXO for genesis
    println!("\n=== Step 5: Get Genesis UTXO ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "list-utxos",
            "--format",
            "compact",
            "--available-only",
            "--confirmed-only",
            "--password",
            password,
        ])
        .expect("Failed to list UTXOs");

    let genesis_utxo = parse_available_utxo(&output).expect("No available UTXO found for genesis");
    println!("✓ Genesis UTXO: {}", genesis_utxo);

    // Step 6: Issue RGB asset
    println!("\n=== Step 6: Issue RGB Asset ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "issue-asset",
            "--ticker",
            "TEST",
            "--name",
            "Test Token",
            "--supply",
            "1000",
            "--precision",
            "0",
            "--genesis-utxo",
            &genesis_utxo,
            "--password",
            password,
        ])
        .expect("Failed to issue asset");

    let contract_id = parse_contract_id(&output).expect("Failed to parse contract ID");
    println!("✓ Asset issued: {}", contract_id);

    // Step 7: Check Alice's initial balance
    println!("\n=== Step 7: Check Alice's Initial Balance ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "rgb-balance",
            "--password",
            password,
        ])
        .expect("Failed to get Alice's balance");

    let alice_balance = parse_rgb_balance(&output).expect("Failed to parse Alice's balance");
    assert_eq!(alice_balance, 1000, "Alice should have 1000 tokens");
    println!("✓ Alice's balance: {} TEST", alice_balance);

    // Step 8: Create Bob's wallet
    println!("\n=== Step 8: Create Bob's Wallet ===");
    let output = cli
        .run(&["wallet", "create", &bob_wallet, "--password", password])
        .expect("Failed to create Bob's wallet");
    assert!(
        output.contains("created successfully"),
        "Bob wallet creation failed"
    );
    println!("✓ Bob's wallet created");

    // Step 9: Get Bob's address and fund
    println!("\n=== Step 9: Fund Bob's Wallet ===");
    let output = cli
        .run(&[
            "--wallet",
            &bob_wallet,
            "get-addresses",
            "--count",
            "1",
            "--password",
            password,
        ])
        .expect("Failed to get Bob's address");

    let bob_address = parse_address(&output).expect("Failed to parse Bob's address");
    println!("  Bob's address: {}", bob_address);

    bitcoin
        .mine_blocks(10, &bob_address)
        .expect("Failed to mine blocks for Bob");
    println!("✓ Mined 10 blocks to Bob's address");

    std::thread::sleep(std::time::Duration::from_secs(3));

    cli.run(&["--wallet", &bob_wallet, "sync", "--password", password])
        .expect("Failed to sync Bob's wallet");
    println!("✓ Bob's wallet synced");

    // Step 10: Export genesis consignment
    println!("\n=== Step 10: Export Genesis Consignment ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "export-genesis",
            "--contract-id",
            &contract_id,
            "--password",
            password,
        ])
        .expect("Failed to export genesis");

    let genesis_path = output
        .lines()
        .find(|line| line.contains("Location:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("Failed to parse genesis consignment path");

    assert!(
        std::path::Path::new(genesis_path).exists(),
        "Genesis file not found"
    );
    println!("✓ Genesis exported to: {}", genesis_path);

    // Step 11: Bob imports genesis
    println!("\n=== Step 11: Bob Imports Genesis ===");
    let output = cli
        .run(&[
            "--wallet",
            &bob_wallet,
            "accept-consignment",
            "--consignment-path",
            genesis_path,
            "--password",
            password,
        ])
        .expect("Failed to import genesis");

    assert!(
        output.contains("accepted successfully"),
        "Genesis import failed"
    );
    println!("✓ Bob imported genesis");

    // Step 12: Bob generates invoice
    println!("\n=== Step 12: Bob Generates Invoice ===");
    let output = cli
        .run(&[
            "--wallet",
            &bob_wallet,
            "generate-invoice",
            "--contract-id",
            &contract_id,
            "--amount",
            "250",
            "--password",
            password,
        ])
        .expect("Failed to generate invoice");

    let invoice = parse_invoice(&output).expect("Failed to parse invoice");
    println!("✓ Invoice: {}...", &invoice[..50]);

    // CRITICAL: Sync Bob after invoice generation
    println!("\n=== Step 12b: Sync Bob After Invoice ===");
    cli.run(&["--wallet", &bob_wallet, "sync", "--password", password])
        .expect("Failed to sync Bob's wallet");
    println!("✓ Bob synced after invoice generation");

    // Step 13: Get Bob's F1r3fly pubkey
    println!("\n=== Step 13: Get Bob's F1r3fly Pubkey ===");
    let output = cli
        .run(&["--wallet", &bob_wallet, "wallet", "get-f1r3fly-pubkey"])
        .expect("Failed to get Bob's pubkey");

    let bob_pubkey = parse_pubkey(&output).expect("Failed to parse Bob's pubkey");
    println!("✓ Bob's pubkey: {}...", &bob_pubkey[..16]);

    // Step 14: Fund Alice for transfer fee
    println!("\n=== Step 14: Fund Alice for Transfer Fee ===");
    bitcoin
        .send_to_address(&alice_address, "0.001")
        .expect("Failed to send BTC to Alice");
    bitcoin
        .mine_blocks(1, &alice_address)
        .expect("Failed to mine blocks");
    std::thread::sleep(std::time::Duration::from_secs(3));

    cli.run(&["--wallet", &alice_wallet, "sync", "--password", password])
        .expect("Failed to sync Alice's wallet");
    println!("✓ Alice funded and synced");

    // Step 15: Alice sends transfer
    println!("\n=== Step 15: Alice Sends Transfer ===");
    let output = cli
        .run_debug(&[
            "--wallet",
            &alice_wallet,
            "send-transfer",
            "--invoice",
            &invoice,
            "--recipient-pubkey",
            &bob_pubkey,
            "--password",
            password,
        ])
        .expect("Failed to send transfer");

    let consignment_path =
        parse_consignment_path(&output).expect("Failed to parse consignment path");

    assert!(
        std::path::Path::new(&consignment_path).exists(),
        "Consignment file not found"
    );
    println!("✓ Transfer sent, consignment at: {}", consignment_path);

    // Step 16: Mine blocks to confirm transfer
    println!("\n=== Step 16: Confirm Transfer ===");
    bitcoin
        .mine_blocks(1, &alice_address)
        .expect("Failed to mine block");
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Mine second block (matches integration test)
    bitcoin
        .mine_blocks(1, &alice_address)
        .expect("Failed to mine second block");
    std::thread::sleep(std::time::Duration::from_secs(3));

    cli.run(&["--wallet", &alice_wallet, "sync", "--password", password])
        .expect("Failed to sync Alice's wallet");
    println!("✓ Mined 2 blocks and synced Alice");

    // Step 17: Verify Alice's balance after transfer
    println!("\n=== Step 17: Verify Alice's Balance After Transfer ===");
    let output = cli
        .run(&[
            "--wallet",
            &alice_wallet,
            "rgb-balance",
            "--password",
            password,
        ])
        .expect("Failed to get Alice's balance");

    let alice_balance_after = parse_rgb_balance(&output).expect("Failed to parse Alice's balance");
    println!(
        "  Alice's balance after transfer: {} TEST",
        alice_balance_after
    );

    if alice_balance_after != 750 {
        eprintln!(
            "⚠ WARNING: Alice should have 750 TEST, got {}",
            alice_balance_after
        );
        eprintln!("  This suggests the transfer didn't complete on F1r3node");
    } else {
        println!("✓ Alice's balance correct (750 TEST)");
    }

    // Step 18: Sync Bob before accepting
    println!("\n=== Step 18: Sync Bob Before Accepting ===");
    cli.run(&["--wallet", &bob_wallet, "sync", "--password", password])
        .expect("Failed to sync Bob's wallet");
    println!("✓ Bob synced before accepting");

    // Step 19: Bob accepts consignment
    println!("\n=== Step 19: Bob Accepts Consignment ===");
    let output = cli
        .run_debug(&[
            "--wallet",
            &bob_wallet,
            "accept-consignment",
            "--consignment-path",
            &consignment_path,
            "--password",
            password,
        ])
        .expect("Failed to accept consignment");

    assert!(
        output.contains("accepted successfully"),
        "Consignment acceptance failed"
    );
    println!("✓ Bob accepted consignment");

    // Step 20: Sync Bob after accepting
    println!("\n=== Step 20: Sync Bob After Accepting ===");
    cli.run(&["--wallet", &bob_wallet, "sync", "--password", password])
        .expect("Failed to sync Bob's wallet");
    println!("✓ Bob synced after accepting");

    // Step 21: Check Bob's balance (with retries)
    println!("\n=== Step 21: Check Bob's Balance (with retries) ===");
    let max_attempts = 5;
    let mut bob_balance = 0;

    for attempt in 1..=max_attempts {
        println!("  Attempt {}/{}...", attempt, max_attempts);

        // Sync
        cli.run(&["--wallet", &bob_wallet, "sync", "--password", password])
            .expect("Failed to sync Bob's wallet");

        // Check balance
        let output = if attempt == 1 {
            cli.run_debug(&[
                "--wallet",
                &bob_wallet,
                "rgb-balance",
                "--password",
                password,
            ])
            .expect("Failed to get Bob's balance")
        } else {
            cli.run(&[
                "--wallet",
                &bob_wallet,
                "rgb-balance",
                "--password",
                password,
            ])
            .expect("Failed to get Bob's balance")
        };

        bob_balance = parse_rgb_balance(&output).unwrap_or(0);

        if bob_balance == 250 {
            println!("✓ Bob's balance correct: {} TEST", bob_balance);
            break;
        }

        if attempt < max_attempts {
            println!("    Balance: {} (expected 250), retrying...", bob_balance);
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    // Final assertions
    println!("\n=== Final Verification ===");
    assert_eq!(
        alice_balance_after, 750,
        "Alice should have 750 TEST remaining"
    );
    assert_eq!(bob_balance, 250, "Bob should have received 250 TEST");
    println!("✓ Transfer complete:");
    println!("  - Alice: 750 TEST");
    println!("  - Bob: 250 TEST");
    println!("  - Total: 1000 TEST (conserved)");
}
