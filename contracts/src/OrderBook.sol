// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import {IRiscZeroVerifier} from "risc0/IRiscZeroVerifier.sol";
import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {BoundlessMarketCallback} from "boundless/BoundlessMarketCallback.sol";
import {Steel} from "steel/Steel.sol";
import {IOrderBook} from "./IOrderBook.sol";

/// @title OrderBook - ZKVM-verified limit order book with ERC20 token swaps
/// @notice Executes order matches proven by RISC Zero ZKVM via Boundless Market
/// @dev Uses UTXO model for stateless ZKVM operation
contract OrderBook is IOrderBook, BoundlessMarketCallback {
    using SafeERC20 for IERC20;

    /// @notice ERC20 token A (base token)
    IERC20 public immutable ASSET_A;

    /// @notice ERC20 token B (quote token)
    IERC20 public immutable ASSET_B;

    /// @notice Current batch index (incremented after each batch execution)
    uint64 public currentBatchIndex;

    /// @notice Merkle root of valid UTXOs
    bytes32 public utxoMerkleRoot;

    /// @notice Mapping of valid UTXO IDs (kept for backwards compatibility)
    mapping(bytes32 => bool) public validUtxos;

    /// @notice Fill data struct from journal
    struct FillData {
        bytes32 makerUtxoId;
        bytes32 takerUtxoId;
        uint64 price;
        uint64 quantity;
        address maker;
        address taker;
        bool makerIsSeller;
    }

    /// @notice UTXO struct from journal
    struct UtxoData {
        bytes32 id;
        uint8 side; // 0 = Buy, 1 = Sell
        uint64 price;
        uint64 quantity;
        address owner;
        uint64 nonce;
        uint64 expiryBatch;
    }

    /// @notice Journal struct from ZKVM (includes Steel commitment)
    struct Journal {
        Steel.Commitment steelCommitment;
        uint64 batchIndex;
        FillData[] fills;
        UtxoData[] newUtxos;
        bytes32[] consumedUtxoIds;
        bytes32 newUtxoMerkleRoot;
    }

    /// @notice Constructor
    /// @param verifier RISC Zero verifier contract address
    /// @param boundlessMarket The BoundlessMarket contract address
    /// @param imageId Image ID of the order book guest program
    /// @param _assetA ERC20 token A (base token)
    /// @param _assetB ERC20 token B (quote token)
    constructor(
        IRiscZeroVerifier verifier,
        address boundlessMarket,
        bytes32 imageId,
        IERC20 _assetA,
        IERC20 _assetB
    ) BoundlessMarketCallback(verifier, boundlessMarket, imageId) {
        ASSET_A = _assetA;
        ASSET_B = _assetB;
        currentBatchIndex = 0;
    }

    /// @notice Internal handler for proof delivery from Boundless Market
    /// @param journalData The ABI-encoded Journal from ZKVM
    function _handleProof(bytes32, bytes calldata journalData, bytes calldata) internal override {
        // Decode the journal
        Journal memory journal = abi.decode(journalData, (Journal));

        // Validate the Steel commitment to ensure the proof is based on valid chain state
        require(Steel.validateCommitment(journal.steelCommitment), "OrderBook: invalid Steel commitment");

        // Verify batch index matches (replay protection)
        require(journal.batchIndex == currentBatchIndex, "OrderBook: invalid batch index");

        // Process consumed UTXOs
        for (uint256 i = 0; i < journal.consumedUtxoIds.length; i++) {
            bytes32 utxoId = journal.consumedUtxoIds[i];
            if (validUtxos[utxoId]) {
                validUtxos[utxoId] = false;
                emit UTXOConsumed(utxoId);
            }
        }

        // Process fills - execute ERC20 transfers
        for (uint256 i = 0; i < journal.fills.length; i++) {
            FillData memory fill = journal.fills[i];
            _executeFill(fill);
        }

        // Process new UTXOs
        for (uint256 i = 0; i < journal.newUtxos.length; i++) {
            bytes32 utxoId = journal.newUtxos[i].id;
            validUtxos[utxoId] = true;
            emit UTXOCreated(utxoId);
        }

        // Update UTXO Merkle root
        utxoMerkleRoot = journal.newUtxoMerkleRoot;

        // Increment batch index
        currentBatchIndex++;

        emit BatchExecuted(journal.batchIndex, journal.fills.length);
    }

    /// @notice Execute a single fill - transfer tokens between maker and taker
    /// @param fill The fill to execute
    function _executeFill(FillData memory fill) internal {
        // Calculate amounts
        // price is in AssetB per AssetA
        // quantity is amount of AssetA
        uint256 assetAAmount = fill.quantity;
        uint256 assetBAmount = uint256(fill.price) * uint256(fill.quantity);

        if (fill.makerIsSeller) {
            // Maker is selling AssetA, Taker is buying AssetA
            // Maker sends AssetA to Taker
            // Taker sends AssetB to Maker
            ASSET_A.safeTransferFrom(fill.maker, fill.taker, assetAAmount);
            ASSET_B.safeTransferFrom(fill.taker, fill.maker, assetBAmount);
        } else {
            // Maker is buying AssetA, Taker is selling AssetA
            // Taker sends AssetA to Maker
            // Maker sends AssetB to Taker
            ASSET_A.safeTransferFrom(fill.taker, fill.maker, assetAAmount);
            ASSET_B.safeTransferFrom(fill.maker, fill.taker, assetBAmount);
        }

        emit Fill(
            fill.makerUtxoId,
            fill.takerUtxoId,
            fill.price,
            fill.quantity,
            fill.maker,
            fill.taker,
            fill.makerIsSeller
        );
    }

    /// @inheritdoc IOrderBook
    function isUtxoValid(bytes32 utxoId) external view returns (bool) {
        return validUtxos[utxoId];
    }

    /// @inheritdoc IOrderBook
    function assetA() external view returns (address) {
        return address(ASSET_A);
    }

    /// @inheritdoc IOrderBook
    function assetB() external view returns (address) {
        return address(ASSET_B);
    }
}
