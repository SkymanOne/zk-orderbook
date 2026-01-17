use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolValue;
use anyhow::{Context, Result};
use boundless_market::{
    request_builder::RequirementParams, Client, Deployment, GuestEnv, StorageProviderConfig,
};
use clap::Parser;
use csv::ReaderBuilder;
use guests::ORDER_BOOK_ELF;
use orderbook::{
    build_utxo_merkle_tree, generate_utxo_proof, BatchInput, Order, Side, SolJournal, Utxo,
    UtxoWithProof,
};
use risc0_steel::{
    ethereum::{EthEvmEnv, ETH_SEPOLIA_CHAIN_SPEC},
    Contract,
};
use serde::{Deserialize, Serialize};
use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};
use url::Url;

// Define the OrderBook contract interface for Steel calls
alloy::sol! {
    #[sol(rpc)]
    interface IOrderBook {
        function utxoMerkleRoot() external view returns (bytes32);
        function currentBatchIndex() external view returns (uint64);
    }
}

/// Order Book ZKVM Host CLI - Boundless Market Edition
#[derive(Parser, Debug)]
#[clap(author, version, about = "Order Book ZKVM Prover via Boundless Market")]
struct Args {
    /// Path to CSV file containing new orders
    #[clap(short, long, env = "ORDERS", default_value = "orders.csv")]
    orders: PathBuf,

    /// Path to JSON file containing existing UTXOs
    #[clap(short, long, env = "UTXO_FILE", default_value = "utxos.json")]
    utxo_file: Option<PathBuf>,

    /// URL of the Ethereum RPC endpoint
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: Url,

    /// Private key used to interact with contracts and Boundless Market
    #[clap(long, env = "PRIVATE_KEY")]
    private_key: PrivateKeySigner,

    /// OrderBook contract address
    #[clap(long, env = "ORDER_BOOK_ADDRESS")]
    order_book: Address,

    /// Configuration for the StorageProvider to use for uploading programs and inputs
    #[clap(flatten, next_help_heading = "Storage Provider")]
    storage_config: StorageProviderConfig,

    /// Boundless Market deployment configuration
    #[clap(flatten, next_help_heading = "Boundless Market Deployment")]
    deployment: Option<Deployment>,
}

/// Serializable UTXO for JSON storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableUtxo {
    id: String,
    side: String,
    price: u64,
    quantity: u64,
    owner: String,
    nonce: u64,
    expiry_batch: u64,
}

impl From<&Utxo> for SerializableUtxo {
    fn from(utxo: &Utxo) -> Self {
        SerializableUtxo {
            id: format!("0x{}", hex::encode(utxo.id)),
            side: match utxo.order.side {
                Side::Buy => "buy".to_string(),
                Side::Sell => "sell".to_string(),
            },
            price: utxo.order.price,
            quantity: utxo.order.quantity,
            owner: format!("{}", utxo.order.owner),
            nonce: utxo.order.nonce,
            expiry_batch: utxo.order.expiry_batch,
        }
    }
}

impl TryFrom<&SerializableUtxo> for Utxo {
    type Error = anyhow::Error;

    fn try_from(s: &SerializableUtxo) -> Result<Self, Self::Error> {
        let order = Order {
            side: match s.side.as_str() {
                "buy" | "Buy" | "BUY" => Side::Buy,
                "sell" | "Sell" | "SELL" => Side::Sell,
                _ => anyhow::bail!("Invalid side: {}", s.side),
            },
            price: s.price,
            quantity: s.quantity,
            owner: s.owner.parse()?,
            nonce: s.nonce,
            expiry_batch: s.expiry_batch,
        };

        // Always compute ID from order data to ensure consistency
        Ok(Utxo::new(order))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::from_str("info")?.into())
                .from_env_lossy(),
        )
        .init();

    match dotenvy::dotenv() {
        Ok(path) => tracing::debug!("Loaded environment variables from {:?}", path),
        Err(e) if e.not_found() => tracing::debug!("No .env file found"),
        Err(e) => anyhow::bail!("failed to load .env file: {}", e),
    }

    let args = Args::parse();
    run(args).await
}

/// Main logic which creates the Boundless client, prepares inputs, and submits the proof request
async fn run(args: Args) -> Result<()> {
    // Read batch size from environment (default: 10)
    let batch_size: usize = std::env::var("BATCH_SIZE")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .context("Invalid BATCH_SIZE")?;

    tracing::info!("Batch size: {}", batch_size);
    tracing::info!("OrderBook contract: {}", args.order_book);

    // Create a Boundless client from the provided parameters
    let client = Client::builder()
        .with_rpc_url(args.rpc_url.clone())
        .with_deployment(args.deployment)
        .with_storage_provider_config(&args.storage_config)?
        .with_private_key(args.private_key)
        .build()
        .await
        .context("failed to build boundless client")?;

    // Load existing UTXOs from JSON file if provided
    let existing_utxos = if let Some(ref utxo_path) = args.utxo_file {
        if utxo_path.exists() {
            let file = File::open(utxo_path)?;
            let reader = BufReader::new(file);
            let serializable: Vec<SerializableUtxo> = serde_json::from_reader(reader)?;
            serializable
                .iter()
                .map(|s| Utxo::try_from(s))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    tracing::info!("Loaded {} existing UTXOs", existing_utxos.len());

    // Parse new orders from CSV
    let new_orders = parse_orders_csv(&args.orders, batch_size)?;
    tracing::info!("Parsed {} new orders", new_orders.len());

    // Create Steel EVM environment for on-chain state verification
    tracing::info!("Creating Steel EVM environment...");
    let mut evm_env = EthEvmEnv::builder()
        .rpc(args.rpc_url.as_str().parse()?)
        .chain_spec(&ETH_SEPOLIA_CHAIN_SPEC)
        .build()
        .await?;

    // Preflight: query on-chain state via Steel
    let mut contract = Contract::preflight(args.order_book, &mut evm_env);

    let on_chain_merkle_root = contract
        .call_builder(&IOrderBook::utxoMerkleRootCall {})
        .call()
        .await?;
    let on_chain_batch_index = contract
        .call_builder(&IOrderBook::currentBatchIndexCall {})
        .call()
        .await?;

    tracing::info!("On-chain batch index: {}", on_chain_batch_index);
    tracing::info!(
        "On-chain UTXO Merkle root: 0x{}",
        hex::encode(on_chain_merkle_root)
    );

    // Build Merkle tree and proofs for existing UTXOs
    let (tree, computed_root) = build_utxo_merkle_tree(&existing_utxos);

    // Verify computed root matches on-chain root (for first batch with no UTXOs, both are zero)
    if existing_utxos.is_empty() {
        tracing::info!("First batch - no existing UTXOs to verify");
    } else {
        assert_eq!(
            computed_root, on_chain_merkle_root,
            "Computed Merkle root does not match on-chain root"
        );
        tracing::info!("Merkle root verified!");
    }

    // Build UTXOs with proofs
    let existing_utxos_with_proofs: Vec<UtxoWithProof> = existing_utxos
        .iter()
        .enumerate()
        .map(|(i, utxo)| {
            let proof_hashes = generate_utxo_proof(&tree, i).unwrap_or_default();
            UtxoWithProof {
                utxo: utxo.clone(),
                proof_hashes,
                leaf_index: i,
            }
        })
        .collect();

    // Create batch input
    let batch_input = BatchInput {
        batch_index: on_chain_batch_index,
        utxo_merkle_root: on_chain_merkle_root,
        existing_utxos_with_proofs,
        new_orders,
    };
    let input_bytes = batch_input.to_sol().abi_encode();

    tracing::info!("Preparing proof request for Boundless Market...");

    // Convert Steel environment to input for guest
    let evm_input = evm_env.into_input().await?;

    // Build guest environment with all inputs
    // The guest reads: evm_input, order_book_address, input_bytes
    let guest_env = GuestEnv::builder()
        .write(&evm_input)?
        .write(&args.order_book)?
        .write(&input_bytes)?;

    // Create a request with a callback to the OrderBook contract
    let request = client
        .new_request()
        .with_program(ORDER_BOOK_ELF)
        .with_env(guest_env)
        // Add the callback to the OrderBook contract
        .with_requirements(
            RequirementParams::builder()
                .callback_address(args.order_book)
                .callback_gas_limit(15_500_000), // Higher gas limit for order execution
        );

    // Submit the request to the blockchain
    let (request_id, expires_at) = client.submit_onchain(request).await?;
    tracing::info!(
        "Submitted proof request {:x} with callback to {}",
        request_id,
        args.order_book
    );

    // Wait for the request to be fulfilled
    tracing::info!("Waiting for request {:x} to be fulfilled...", request_id);
    let fulfillment = client
        .wait_for_request_fulfillment(
            request_id,
            Duration::from_secs(10), // check every 10 seconds
            expires_at,
        )
        .await?;
    tracing::info!("Request {:x} fulfilled!", request_id);

    // Extract journal from fulfillment and decode
    let fulfillment_data = fulfillment
        .data()
        .context("failed to decode fulfillment data")?;
    let journal_bytes = fulfillment_data
        .journal()
        .context("fulfillment has no journal")?;
    let journal = <SolJournal>::abi_decode(journal_bytes).context("failed to decode journal")?;

    tracing::info!("=== Batch Execution Summary ===");
    tracing::info!("Batch index: {}", journal.batchIndex);
    tracing::info!("Fills executed: {}", journal.fills.len());
    tracing::info!("New UTXOs created: {}", journal.newUtxos.len());
    tracing::info!("UTXOs consumed: {}", journal.consumedUtxoIds.len());
    tracing::info!(
        "New UTXO Merkle root: 0x{}",
        hex::encode(journal.newUtxoMerkleRoot)
    );

    // Print fill details
    for (i, fill) in journal.fills.iter().enumerate() {
        tracing::info!(
            "Fill {}: {} -> {} @ {} for {} units",
            i,
            fill.maker,
            fill.taker,
            fill.price,
            fill.quantity
        );
    }

    // Save new UTXOs to file for next batch
    if let Some(ref utxo_path) = args.utxo_file {
        let new_utxos: Vec<SerializableUtxo> = journal
            .newUtxos
            .iter()
            .map(|sol_utxo| {
                let utxo = Utxo::from(sol_utxo);
                SerializableUtxo::from(&utxo)
            })
            .collect();
        let json = serde_json::to_string_pretty(&new_utxos)?;
        std::fs::write(utxo_path, json)?;
        tracing::info!("Saved {} new UTXOs to {:?}", new_utxos.len(), utxo_path);
    }

    tracing::info!("Order book batch processed successfully via Boundless Market!");

    Ok(())
}

/// Parse orders from CSV file
fn parse_orders_csv(path: &PathBuf, limit: usize) -> Result<Vec<Order>> {
    let file = File::open(path)?;
    let mut reader = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut orders = Vec::new();
    // good enough for PoC
    // TODO: use on-chain nonce
    let mut nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos() as u64;

    for result in reader.records().take(limit) {
        let record = result?;

        let side = match record.get(0).context("Missing side")? {
            "buy" | "Buy" | "BUY" => Side::Buy,
            "sell" | "Sell" | "SELL" => Side::Sell,
            s => anyhow::bail!("Invalid side: {}", s),
        };

        let price: u64 = record
            .get(1)
            .context("Missing price")?
            .parse()
            .context("Invalid price")?;

        let quantity: u64 = record
            .get(2)
            .context("Missing quantity")?
            .parse()
            .context("Invalid quantity")?;

        let owner: Address = record
            .get(3)
            .context("Missing owner")?
            .parse()
            .context("Invalid owner address")?;

        let expiry_batch: u64 = record
            .get(4)
            .context("Missing expiry_batch")?
            .parse()
            .context("Invalid expiry_batch")?;

        orders.push(Order {
            side,
            price,
            quantity,
            owner,
            nonce,
            expiry_batch,
        });

        nonce += 1;
    }

    Ok(orders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::sol_types::SolValue;
    use risc0_zkvm::{default_executor, ExecutorEnv};

    /// Benchmark test that measures ZKVM cycle count for order matching
    /// Uses the same 8 orders as in orders.csv
    ///
    /// Run with: cargo test --release benchmark_cycle_count -- --nocapture
    /// Requires RPC_URL and ORDER_BOOK_ADDRESS environment variables
    #[tokio::test]
    async fn benchmark_cycle_count() -> Result<()> {
        // Load environment
        dotenvy::dotenv().ok();

        let rpc_url: Url = std::env::var("RPC_URL")
            .context("RPC_URL not set")?
            .parse()?;
        let order_book_address: Address = std::env::var("ORDER_BOOK_ADDRESS")
            .context("ORDER_BOOK_ADDRESS not set")?
            .parse()?;

        println!("Benchmarking with OrderBook: {}", order_book_address);
        println!("RPC URL: {}", rpc_url);

        // Create the same 8 orders as in orders.csv
        let base_nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u64;

        let alice: Address = "0x853e3dC3005b83db47B21d6532F3c5500E970d8F".parse()?;
        let bob: Address = "0xf841c5bba73Fa25AE775B0a3a2D816d06B044070".parse()?;

        let new_orders = vec![
            Order {
                side: Side::Buy,
                price: 105,
                quantity: 100,
                owner: alice,
                nonce: base_nonce,
                expiry_batch: 100,
            },
            Order {
                side: Side::Buy,
                price: 103,
                quantity: 50,
                owner: alice,
                nonce: base_nonce + 1,
                expiry_batch: 100,
            },
            Order {
                side: Side::Buy,
                price: 100,
                quantity: 200,
                owner: alice,
                nonce: base_nonce + 2,
                expiry_batch: 50,
            },
            Order {
                side: Side::Sell,
                price: 99,
                quantity: 75,
                owner: bob,
                nonce: base_nonce + 3,
                expiry_batch: 100,
            },
            Order {
                side: Side::Sell,
                price: 101,
                quantity: 150,
                owner: bob,
                nonce: base_nonce + 4,
                expiry_batch: 100,
            },
            Order {
                side: Side::Sell,
                price: 104,
                quantity: 80,
                owner: bob,
                nonce: base_nonce + 5,
                expiry_batch: 100,
            },
            Order {
                side: Side::Buy,
                price: 102,
                quantity: 60,
                owner: alice,
                nonce: base_nonce + 6,
                expiry_batch: 100,
            },
            Order {
                side: Side::Sell,
                price: 100,
                quantity: 40,
                owner: bob,
                nonce: base_nonce + 7,
                expiry_batch: 100,
            },
        ];

        println!("Created {} orders for benchmark", new_orders.len());

        // Create Steel EVM environment
        let mut evm_env = EthEvmEnv::builder()
            .rpc(rpc_url.as_str().parse()?)
            .chain_spec(&ETH_SEPOLIA_CHAIN_SPEC)
            .build()
            .await?;

        // Preflight: query on-chain state
        let mut contract = Contract::preflight(order_book_address, &mut evm_env);
        let on_chain_merkle_root = contract
            .call_builder(&IOrderBook::utxoMerkleRootCall {})
            .call()
            .await?;
        let on_chain_batch_index = contract
            .call_builder(&IOrderBook::currentBatchIndexCall {})
            .call()
            .await?;

        println!("On-chain batch index: {}", on_chain_batch_index);
        println!(
            "On-chain UTXO Merkle root: 0x{}",
            hex::encode(on_chain_merkle_root)
        );

        // Create batch input (no existing UTXOs for simplicity)
        let batch_input = BatchInput {
            batch_index: on_chain_batch_index,
            utxo_merkle_root: on_chain_merkle_root,
            existing_utxos_with_proofs: vec![],
            new_orders,
        };
        let input_bytes = batch_input.to_sol().abi_encode();

        // Convert Steel environment to input
        let evm_input = evm_env.into_input().await?;

        // Build executor environment
        let env = ExecutorEnv::builder()
            .write(&evm_input)?
            .write(&order_book_address)?
            .write(&input_bytes)?
            .build()?;

        // Run the executor and measure cycles
        println!("\nRunning executor...");
        let exec = default_executor();
        let session = exec.execute(env, ORDER_BOOK_ELF)?;

        let total_cycles = session.cycles();
        let segments = session.segments.len();

        println!("\n=== Benchmark Results ===");
        println!("Total cycles: {}", total_cycles);
        println!("Segments: {}", segments);
        println!("Orders processed: 8");
        println!("Cycles per order: {}", total_cycles / 8);

        Ok(())
    }
}
