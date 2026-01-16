// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

interface IOrderBook {
    /// @notice Fill event emitted when orders are matched
    event Fill(
        bytes32 indexed makerUtxoId,
        bytes32 indexed takerUtxoId,
        uint64 price,
        uint64 quantity,
        address maker,
        address taker,
        bool makerIsSeller
    );

    /// @notice Event emitted when a new UTXO is created
    event UTXOCreated(bytes32 indexed utxoId);

    /// @notice Event emitted when a UTXO is consumed
    event UTXOConsumed(bytes32 indexed utxoId);

    /// @notice Event emitted when a batch is executed
    event BatchExecuted(uint64 indexed batchIndex, uint256 fillCount);

    /// @notice Get the current batch index
    function currentBatchIndex() external view returns (uint64);

    /// @notice Get the current UTXO Merkle root
    function utxoMerkleRoot() external view returns (bytes32);

    /// @notice Check if a UTXO is valid (not consumed)
    function isUtxoValid(bytes32 utxoId) external view returns (bool);

    /// @notice Get the AssetA token address
    function assetA() external view returns (address);

    /// @notice Get the AssetB token address
    function assetB() external view returns (address);
}
