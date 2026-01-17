// SPDX-License-Identifier: Apache-2.0
// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";
import {IRiscZeroVerifier} from "risc0/IRiscZeroVerifier.sol";
import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {OrderBook} from "../src/OrderBook.sol";
import {MockERC20} from "../src/MockERC20.sol";
import {ImageID} from "../src/ImageID.sol";

/// @title Deploy - Deploys OrderBook and mock ERC20 tokens for testing
/// @notice Deploys all contracts and sets up test addresses with token approvals
contract Deploy is Script {
    // Initial token amounts to mint (1 million tokens with 18 decimals)
    uint256 constant INITIAL_MINT = 1_000_000 * 1e18;

    // Approval amount (max uint256 for unlimited)
    uint256 constant APPROVAL_AMOUNT = type(uint256).max;

    function run() external {
        // Load ENV variables
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address verifierAddress = vm.envAddress("VERIFIER_ADDRESS");
        address boundlessMarket = vm.envAddress("BOUNDLESS_MARKET");
        bytes32 imageId = ImageID.ORDER_BOOK_ID;

        // Load demo wallet addresses from env
        address alice = vm.envAddress("ALICE_ADDRESS");
        address bob = vm.envAddress("BOB_ADDRESS");

        // Optional: use existing tokens or deploy new ones
        bool deployNewTokens = vm.envOr("DEPLOY_NEW_TOKENS", true);

        vm.startBroadcast(deployerKey);

        MockERC20 assetA;
        MockERC20 assetB;

        if (deployNewTokens) {
            // Deploy mock ERC20 tokens
            assetA = new MockERC20("Asset A Token", "ASTA", 18);
            assetB = new MockERC20("Asset B Token", "ASTB", 18);

            console2.log("Deployed AssetA (ASTA) to", address(assetA));
            console2.log("Deployed AssetB (ASTB) to", address(assetB));

            // Mint initial tokens to test addresses
            assetA.mint(alice, INITIAL_MINT);
            assetA.mint(bob, INITIAL_MINT);
            assetB.mint(alice, INITIAL_MINT);
            assetB.mint(bob, INITIAL_MINT);

            console2.log("Minted", INITIAL_MINT / 1e18, "tokens to ALICE:", alice);
            console2.log("Minted", INITIAL_MINT / 1e18, "tokens to BOB:", bob);
        } else {
            // Use existing token addresses from env
            assetA = MockERC20(vm.envAddress("ASSET_A"));
            assetB = MockERC20(vm.envAddress("ASSET_B"));

            console2.log("Using existing AssetA:", address(assetA));
            console2.log("Using existing AssetB:", address(assetB));
        }

        // Deploy OrderBook
        IRiscZeroVerifier verifier = IRiscZeroVerifier(verifierAddress);
        OrderBook orderBook =
            new OrderBook(verifier, boundlessMarket, imageId, IERC20(address(assetA)), IERC20(address(assetB)));

        console2.log("Deployed OrderBook to", address(orderBook));
        console2.log("  - AssetA:", address(assetA));
        console2.log("  - AssetB:", address(assetB));
        console2.log("  - Verifier:", verifierAddress);
        console2.log("  - BoundlessMarket:", boundlessMarket);
        console2.logBytes32(imageId);

        vm.stopBroadcast();

        // Set up infinite approvals for ALICE and BOB
        _setupApprovals(address(orderBook), address(assetA), address(assetB));

        console2.log("");
        console2.log("=== Deployment Summary ===");
        console2.log("OrderBook:", address(orderBook));
        console2.log("AssetA:", address(assetA));
        console2.log("AssetB:", address(assetB));
        console2.log("ALICE:", alice);
        console2.log("BOB:", bob);
    }

    function _setupApprovals(address orderBook, address assetA, address assetB) internal {
        // Load test private keys from env
        uint256 aliceKey = vm.envUint("ALICE_PRIVATE_KEY");
        uint256 bobKey = vm.envUint("BOB_PRIVATE_KEY");

        // ALICE approves OrderBook for infinite amount
        vm.startBroadcast(aliceKey);
        IERC20(assetA).approve(orderBook, APPROVAL_AMOUNT);
        IERC20(assetB).approve(orderBook, APPROVAL_AMOUNT);
        vm.stopBroadcast();
        console2.log("ALICE approved OrderBook for infinite amount of both tokens");

        // BOB approves OrderBook for infinite amount
        vm.startBroadcast(bobKey);
        IERC20(assetA).approve(orderBook, APPROVAL_AMOUNT);
        IERC20(assetB).approve(orderBook, APPROVAL_AMOUNT);
        vm.stopBroadcast();
        console2.log("BOB approved OrderBook for infinite amount of both tokens");
    }
}

/// @title DeployLocal - Simplified deployment for local Anvil testing
/// @notice Uses Anvil's default test accounts for easy local testing
contract DeployLocal is Script {
    // Anvil default test accounts
    // Account 0: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    // Account 1: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8
    uint256 constant ANVIL_ACCOUNT_0_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant ANVIL_ACCOUNT_1_KEY = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;

    address constant ALICE = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    address constant BOB = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    uint256 constant INITIAL_MINT = 1_000_000 * 1e18;
    uint256 constant APPROVAL_AMOUNT = type(uint256).max;

    function run() external {
        // Use Anvil's default deployer account
        vm.startBroadcast(ANVIL_ACCOUNT_0_KEY);

        // Deploy mock ERC20 tokens
        MockERC20 assetA = new MockERC20("Asset A Token", "ASTA", 18);
        MockERC20 assetB = new MockERC20("Asset B Token", "ASTB", 18);

        console2.log("Deployed AssetA (ASTA) to", address(assetA));
        console2.log("Deployed AssetB (ASTB) to", address(assetB));

        // Mint tokens to test addresses
        assetA.mint(ALICE, INITIAL_MINT);
        assetA.mint(BOB, INITIAL_MINT);
        assetB.mint(ALICE, INITIAL_MINT);
        assetB.mint(BOB, INITIAL_MINT);

        console2.log("Minted tokens to ALICE and BOB");

        // For local testing, use mock verifier address (deploy separately or use existing)
        // These would typically be deployed by risc0 tooling
        address mockVerifier = vm.envOr("VERIFIER_ADDRESS", address(0x1234));
        address mockBoundlessMarket = vm.envOr("BOUNDLESS_MARKET", address(0x5678));
        bytes32 mockImageId = ImageID.ORDER_BOOK_ID;

        // Deploy OrderBook
        OrderBook orderBook = new OrderBook(
            IRiscZeroVerifier(mockVerifier),
            mockBoundlessMarket,
            mockImageId,
            IERC20(address(assetA)),
            IERC20(address(assetB))
        );

        console2.log("Deployed OrderBook to", address(orderBook));

        // ALICE approves OrderBook (deployer is ALICE in this case)
        assetA.approve(address(orderBook), APPROVAL_AMOUNT);
        assetB.approve(address(orderBook), APPROVAL_AMOUNT);
        console2.log("ALICE approved OrderBook for both tokens");

        vm.stopBroadcast();

        // BOB approves OrderBook
        vm.startBroadcast(ANVIL_ACCOUNT_1_KEY);
        assetA.approve(address(orderBook), APPROVAL_AMOUNT);
        assetB.approve(address(orderBook), APPROVAL_AMOUNT);
        console2.log("BOB approved OrderBook for both tokens");
        vm.stopBroadcast();

        console2.log("");
        console2.log("=== Local Deployment Summary ===");
        console2.log("OrderBook:", address(orderBook));
        console2.log("AssetA:", address(assetA));
        console2.log("AssetB:", address(assetB));
        console2.log("ALICE:", ALICE);
        console2.log("BOB:", BOB);
        console2.log("");
        console2.log("To use with the CLI:");
        console2.log("  export ORDER_BOOK_ADDRESS=", address(orderBook));
        console2.log("  export ASSET_A=", address(assetA));
        console2.log("  export ASSET_B=", address(assetB));
    }
}
