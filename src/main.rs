//! F1r3fly-RGB Wallet CLI
//!
//! Command-line interface for managing Bitcoin wallets with RGB smart contracts support

use clap::Parser;
use f1r3fly_rgb_wallet::cli::args::{Cli, Commands, ConfigAction, WalletAction};
use f1r3fly_rgb_wallet::cli::commands;
use f1r3fly_rgb_wallet::config::{ConfigOverrides, NetworkType};
use std::process;

fn main() {
    let cli = Cli::parse();

    // Parse network string to NetworkType
    let network = cli.network.as_ref().and_then(|n| match n.as_str() {
        "regtest" => Some(NetworkType::Regtest),
        "signet" => Some(NetworkType::Signet),
        "testnet" => Some(NetworkType::Testnet),
        "mainnet" => Some(NetworkType::Mainnet),
        _ => {
            eprintln!(
                "Error: Invalid network '{}'. Use: regtest, signet, testnet, or mainnet",
                n
            );
            process::exit(1);
        }
    });

    // Build config overrides from global arguments
    let overrides = ConfigOverrides {
        network,
        f1r3node_host: cli.f1r3node_host.clone(),
        f1r3node_grpc_port: cli.f1r3node_grpc_port,
        f1r3node_http_port: cli.f1r3node_http_port,
        esplora_url: cli.esplora_url.clone(),
        wallets_dir: cli.data_dir.clone(),
    };

    let result: Result<(), Box<dyn std::error::Error>> = match cli.command {
        Commands::Config { action } => match action {
            ConfigAction::Init { network } => commands::config::init(network).map_err(Into::into),
        },

        Commands::Wallet { action } => match action {
            WalletAction::Create { name, password } => {
                commands::wallet::create(name, password, overrides).map_err(Into::into)
            }

            WalletAction::Import {
                name,
                mnemonic,
                password,
            } => commands::wallet::import(name, mnemonic, password, overrides).map_err(Into::into),

            WalletAction::List => commands::wallet::list(overrides).map_err(Into::into),
        },

        Commands::Sync { password } => {
            commands::bitcoin::sync(cli.wallet, password, overrides).map_err(Into::into)
        }

        Commands::GetBalance { password } => {
            commands::bitcoin::get_balance(cli.wallet, password, overrides).map_err(Into::into)
        }

        Commands::GetAddresses { count, password } => {
            commands::bitcoin::get_addresses(cli.wallet, count, password, overrides)
                .map_err(Into::into)
        }

        Commands::CreateUtxo {
            amount,
            fee_rate,
            password,
        } => commands::bitcoin::create_utxo(cli.wallet, amount, fee_rate, password, overrides)
            .map_err(Into::into),

        Commands::SendBitcoin {
            to,
            amount,
            fee_rate,
            password,
        } => commands::bitcoin::send_bitcoin(cli.wallet, to, amount, fee_rate, password, overrides)
            .map_err(Into::into),

        Commands::IssueAsset {
            ticker,
            name,
            supply,
            precision,
            genesis_utxo,
            password,
        } => {
            match cli.wallet.as_deref() {
                Some(wallet_name) => {
                    match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt
                            .block_on(commands::rgb::issue_asset(
                                wallet_name,
                                &ticker,
                                &name,
                                supply,
                                precision,
                                &genesis_utxo,
                                &password,
                                &overrides,
                            ))
                            .map(|_| ())
                            .map_err(Into::into),
                        Err(e) => Err(format!("Failed to create async runtime: {}", e).into()),
                    }
                }
                None => Err("Wallet name required (use --wallet <name>)".into()),
            }
        }

        Commands::ListAssets { password } => match cli.wallet.as_deref() {
            Some(wallet_name) => commands::rgb::list_assets(wallet_name, &password, &overrides)
                .map(|_| ())
                .map_err(Into::into),
            None => Err("Wallet name required (use --wallet <name>)".into()),
        },

        Commands::RgbBalance {
            contract_id,
            password,
        } => match cli.wallet.as_deref() {
            Some(wallet_name) => match tokio::runtime::Runtime::new() {
                Ok(rt) => rt
                    .block_on(commands::rgb::rgb_balance(
                        wallet_name,
                        contract_id.as_deref(),
                        &password,
                        &overrides,
                    ))
                    .map(|_| ())
                    .map_err(Into::into),
                Err(e) => Err(format!("Failed to create async runtime: {}", e).into()),
            },
            None => Err("Wallet name required (use --wallet <name>)".into()),
        },

        Commands::GetContractInfo {
            contract_id,
            password,
        } => match cli.wallet.as_deref() {
            Some(wallet_name) => {
                commands::rgb::get_contract_info(wallet_name, &contract_id, &password, &overrides)
                    .map(|_| ())
                    .map_err(Into::into)
            }
            None => Err("Wallet name required (use --wallet <name>)".into()),
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
