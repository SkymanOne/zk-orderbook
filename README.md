# ZKVM Order Book

A proof of concept limit order book where order matching is executed off chain inside a RISC Zero ZKVM guest program and verified on chain through Boundless Market.

## Overview

Traditional on chain order books are expensive because every match requires multiple storage writes and token transfers. This PoC moves the matching logic off chain while preserving trustlessness through zero knowledge proofs.

The matching engine runs inside a ZKVM guest program. It takes a batch of orders, runs price-time priority matching, and outputs the fills. A risc0 proof attests that the matching was done correctly. The proof is submitted to the OrderBook smart contract which then executes the ERC20 transfers.

## Architecture

The system has three main components.

**Guest Program** processes batches of orders inside the ZKVM. It verifies Merkle proofs for existing orders, runs the matching algorithm, and commits the results to a journal. Steel is used to read on chain state and ensure the proof is anchored to a specific block.

**OrderBook Contract** receives proofs through Boundless Market callbacks. It validates the Steel commitment, verifies the batch index for replay protection, and executes token transfers for each fill. The contract maintains a Merkle root of all unfilled orders.

**Host Application** coordinates the flow. It reads orders from a CSV file, queries on chain state, builds the guest input with Merkle proofs, and submits proof requests to Boundless Market.

## UTXO Model

Orders are represented as UTXOs. Each order gets a unique ID derived from hashing its fields. When an order is partially filled, the original UTXO is consumed and a new one is created with the remaining quantity. This model allows the ZKVM to operate statelessly since it only needs Merkle proofs to verify existing orders rather than reading the full order book.

## Order Matching

The matching engine implements standard price time priority. Buy orders are sorted by price descending then by nonce ascending. Sell orders are sorted by price ascending then by nonce ascending. Orders cross when the best buy price meets or exceeds the best sell price. The execution price is the maker price. Self trading is prevented by skipping matches where both sides have the same owner.

## Proof Flow

1. Host fetches current batch index and UTXO Merkle root from the contract
2. Host builds Merkle proofs for any existing UTXOs being included
3. Host creates Steel EVM input anchored to current block
4. Guest verifies on chain state matches input via Steel
5. Guest verifies Merkle proofs for existing UTXOs
6. Guest runs matching and outputs fills and new UTXOs
7. Proof is generated and submitted to Boundless Market
8. Boundless Market calls back to OrderBook contract with a proof and a journal
9. Contract validates proof and executes ERC20 transfers

## Benchmarks

A rough cycle benchmark for 8 orders:
```
Total cycles: 1345806
Segments: 2
Orders processed: 8
Cycles per order: 168225
```

## Running

Set environment variables in a `.env` file, follow `example.env` for guidance.

Deploy contracts.

```bash
just deploy-sepolia
```

Submit a batch of orders.

```bash
cargo run --bin app -- --order-book YOUR_ORDER_BOOK_ADDRESS
```

Run the cycle count benchmark.

```bash
ORDER_BOOK_ADDRESS=YOUR_ADDRESS cargo test --release -p app benchmark_cycle_count -- --nocapture
```

## Limitations

This is a proof of concept with several limitations:

- Nonces are generated from timestamps rather than a proper on chain counter. In production orders would need verifiable unique identifiers.
- The Merkle tree implementation stores all UTXOs in memory. A production system would need a persistent indexed data structure.
- There is no fee mechanism. Real order books charge maker and taker fees.
- Batch size is fixed. Dynamic batching based on gas costs and proof generation time would be needed.
- The system only supports a single orderbook instance. Supporting multiple pairs would require additional contract logic such as factory. The guest code should also be adapted to support.
