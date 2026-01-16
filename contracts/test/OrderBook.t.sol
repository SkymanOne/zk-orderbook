// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {RiscZeroCheats} from "risc0/test/RiscZeroCheats.sol";
import {RiscZeroMockVerifier} from "risc0/test/RiscZeroMockVerifier.sol";
import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ERC20} from "openzeppelin/contracts/token/ERC20/ERC20.sol";
import {OrderBook} from "../src/OrderBook.sol";

/// @notice Simple mock ERC20 for testing
contract MockERC20 is ERC20 {
    constructor(string memory name, string memory symbol) ERC20(name, symbol) {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

contract OrderBookTest is RiscZeroCheats, Test {
    OrderBook public orderBook;
    RiscZeroMockVerifier public verifier;
    MockERC20 public assetA;
    MockERC20 public assetB;
    address public boundlessMarket;
    bytes32 public imageId;

    function setUp() public {
        verifier = new RiscZeroMockVerifier(0);
        boundlessMarket = makeAddr("boundlessMarket");
        imageId = bytes32(uint256(1));

        assetA = new MockERC20("Asset A", "ASTA");
        assetB = new MockERC20("Asset B", "ASTB");

        orderBook = new OrderBook(
            verifier,
            boundlessMarket,
            imageId,
            IERC20(address(assetA)),
            IERC20(address(assetB))
        );
    }

    function test_InitialState() public view {
        assertEq(orderBook.currentBatchIndex(), 0);
        assertEq(orderBook.utxoMerkleRoot(), bytes32(0));
        assertEq(orderBook.assetA(), address(assetA));
        assertEq(orderBook.assetB(), address(assetB));
    }

    function test_IsUtxoValid() public view {
        bytes32 randomUtxoId = bytes32(uint256(12345));
        assertFalse(orderBook.isUtxoValid(randomUtxoId));
    }
}
