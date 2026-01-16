use alloy_primitives::Address;
use alloy_sol_types::{sol, SolValue};
use orderbook::{match_orders, BatchInput, SolBatchInput};
use risc0_steel::{ethereum::EthEvmInput, ethereum::ETH_SEPOLIA_CHAIN_SPEC, Contract};
use risc0_zkvm::guest::env;

// Define the OrderBook contract interface for Steel calls
sol! {
    interface IOrderBook {
        function utxoMerkleRoot() external view returns (bytes32);
        function currentBatchIndex() external view returns (uint64);
    }
}

fn main() {
    // Read the Steel EVM input
    let evm_input: EthEvmInput = env::read();

    // Read the OrderBook contract address
    let order_book_address: Address = env::read();

    // Read the ABI-encoded batch input
    let input_bytes: Vec<u8> = env::read();
    let sol_input = <SolBatchInput>::abi_decode(&input_bytes).unwrap();

    // Create Steel environment and contract
    let evm_env = evm_input.into_env(&ETH_SEPOLIA_CHAIN_SPEC);
    let contract = Contract::new(order_book_address, &evm_env);

    // Query on-chain state
    let on_chain_merkle_root = contract
        .call_builder(&IOrderBook::utxoMerkleRootCall {})
        .call();
    let on_chain_batch_index = contract
        .call_builder(&IOrderBook::currentBatchIndexCall {})
        .call();

    // Verify input matches on-chain state
    assert_eq!(
        sol_input.utxoMerkleRoot, on_chain_merkle_root,
        "UTXO Merkle root mismatch"
    );
    assert_eq!(
        sol_input.batchIndex, on_chain_batch_index,
        "Batch index mismatch"
    );

    // Convert to internal types
    let input = BatchInput::from_sol(&sol_input);

    // Run the matching engine (this also verifies Merkle proofs for UTXOs)
    let output = match_orders(input);

    // Get the Steel commitment and create journal
    let commitment = evm_env.into_commitment();
    let journal = output.to_journal(commitment);

    // Commit the journal (ABI-encoded for Solidity)
    env::commit_slice(&journal.abi_encode());
}
