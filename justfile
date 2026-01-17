set dotenv-load

deploy-sepolia:
    forge script contracts/scripts/Deploy.s.sol:Deploy \
        --chain sepolia \
        --rpc-url $RPC_URL \
        --broadcast \
        --verify \
        -vvvv

run:
    cargo run --bin app -- --boundless-market-address $BOUNDLESS_MARKET --set-verifier-address $VERIFIER_ADDRESS 