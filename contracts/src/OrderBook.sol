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

    /// @notice Merkle root of valid Order UTXOs
    bytes32 public utxoMerkleRoot;

    /// @notice Mapping to track verified proofs.
    /// @dev This is used to prevent a callback is called more than once with the same proof.
    mapping(bytes32 => bool) public verified;

    error AlreadyVerified();

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
    constructor(IRiscZeroVerifier verifier, address boundlessMarket, bytes32 imageId, IERC20 _assetA, IERC20 _assetB)
        BoundlessMarketCallback(verifier, boundlessMarket, imageId)
    {
        ASSET_A = _assetA;
        ASSET_B = _assetB;
        currentBatchIndex = 0;
    }

    /// @notice Internal handler for proof delivery from Boundless Market
    /// @param journalData The ABI-encoded Journal from ZKVM
    function _handleProof(bytes32, bytes calldata journalData, bytes calldata seal) internal override {
        // Since a callback can be triggered by any requestor sending a valid request to the Boundless Market,
        // we need to perform some checks on the proof before proceeding.
        // First, the validation of the proof (e.g., seal is valid, the caller of the callback is the BoundlessMarket)
        // is done in the parent contract, the `BoundlessMarketCallback`.
        // Here we can add additional checks if needed.
        // For example, we can check if the proof has already been verified,
        // so that the same proof cannot be used more than once to run the callback logic.
        // can't use assembly since data is of variable size. May optimise later
        bytes32 journalAndSeal = keccak256(abi.encode(journalData, seal));
        if (verified[journalAndSeal]) {
            revert AlreadyVerified();
        }
        // Mark the proof as verified.
        verified[journalAndSeal] = true;

        // Decode the journal
        Journal memory journal = abi.decode(journalData, (Journal));

        // Validate the Steel commitment to ensure the proof is based on valid chain state
        require(Steel.validateCommitment(journal.steelCommitment), "OrderBook: invalid Steel commitment");

        // Verify batch index matches (replay protection)
        require(journal.batchIndex == currentBatchIndex, "OrderBook: invalid batch index");

        // Emit events for consumed UTXOs
        for (uint256 i = 0; i < journal.consumedUtxoIds.length; i++) {
            emit UTXOConsumed(journal.consumedUtxoIds[i]);
        }

        // Process fills - execute ERC20 transfers
        for (uint256 i = 0; i < journal.fills.length; i++) {
            FillData memory fill = journal.fills[i];
            _executeFill(fill);
        }

        // Emit events for new UTXOs
        for (uint256 i = 0; i < journal.newUtxos.length; i++) {
            emit UTXOCreated(journal.newUtxos[i].id);
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
            fill.makerUtxoId, fill.takerUtxoId, fill.price, fill.quantity, fill.maker, fill.taker, fill.makerIsSeller
        );
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
