use alloy_primitives::{Address, FixedBytes};
use alloy_sol_types::sol;
use rs_merkle::{algorithms::Sha256 as MerkleSha256, MerkleProof, MerkleTree};
use sha2::{Digest, Sha256};

// Re-export Commitment so sol! macro can resolve Steel.Commitment
#[allow(non_snake_case)]
mod Steel {
    pub use risc0_steel::Commitment;
}

// Re-export Commitment for external use
pub use risc0_steel::Commitment;

/// Re-export MerkleProof for external use
pub use rs_merkle::MerkleProof as RsMerkleProof;

/// Order side: Buy or Sell
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// A limit order
#[derive(Debug, Clone)]
pub struct Order {
    /// Buy or Sell
    pub side: Side,
    /// Price in AssetB per AssetA (e.g., 100 means 100 AssetB for 1 AssetA)
    pub price: u64,
    /// Quantity of AssetA to trade
    pub quantity: u64,
    /// Owner's Ethereum address
    pub owner: Address,
    /// Unique nonce for this order (used for ordering and UTXO ID generation)
    pub nonce: u64,
    /// Batch number after which this order expires
    pub expiry_batch: u64,
}

impl Order {
    /// Compute the UTXO ID for this order (hash of all fields)
    pub fn compute_utxo_id(&self) -> FixedBytes<32> {
        let mut hasher = Sha256::new();
        hasher.update([self.side as u8]);
        hasher.update(self.price.to_le_bytes());
        hasher.update(self.quantity.to_le_bytes());
        hasher.update(self.owner.as_slice());
        hasher.update(self.nonce.to_le_bytes());
        hasher.update(self.expiry_batch.to_le_bytes());
        FixedBytes::from_slice(&hasher.finalize())
    }
}

/// A UTXO representing an unfilled or partially filled order
#[derive(Debug, Clone)]
pub struct Utxo {
    /// Unique identifier (hash of original order data)
    pub id: FixedBytes<32>,
    /// The order data
    pub order: Order,
}

impl Utxo {
    /// Create a new UTXO from an order
    pub fn new(order: Order) -> Self {
        let id = order.compute_utxo_id();
        Self { id, order }
    }

    /// Check if this UTXO is expired at the given batch
    pub fn is_expired(&self, current_batch: u64) -> bool {
        self.order.expiry_batch < current_batch
    }
}

/// A UTXO with its Merkle proof for on-chain verification
#[derive(Debug, Clone)]
pub struct UtxoWithProof {
    /// The UTXO data
    pub utxo: Utxo,
    /// Merkle proof (hashes in the proof path)
    pub proof_hashes: Vec<[u8; 32]>,
    /// Index of this UTXO in the Merkle tree
    pub leaf_index: usize,
}

impl UtxoWithProof {
    /// Verify this UTXO against a Merkle root
    pub fn verify(&self, root: &FixedBytes<32>, total_leaves: usize) -> bool {
        let proof = MerkleProof::<MerkleSha256>::new(self.proof_hashes.clone());
        // Use UTXO ID directly as leaf (it's already a hash)
        let leaf: [u8; 32] = self.utxo.id.0;
        proof.verify(
            root.as_slice().try_into().unwrap_or([0u8; 32]),
            &[self.leaf_index],
            &[leaf],
            total_leaves,
        )
    }
}

/// Compute Merkle root from a list of UTXO IDs
pub fn compute_utxo_merkle_root(utxo_ids: &[FixedBytes<32>]) -> FixedBytes<32> {
    if utxo_ids.is_empty() {
        return FixedBytes::ZERO;
    }

    // Use UTXO IDs directly as leaves (they're already hashes)
    let leaves: Vec<[u8; 32]> = utxo_ids.iter().map(|id| id.0).collect();

    let tree = MerkleTree::<MerkleSha256>::from_leaves(&leaves);
    let root = tree.root().unwrap_or([0u8; 32]);
    FixedBytes::from_slice(&root)
}

/// Build a Merkle tree from UTXOs and return the tree for proof generation
pub fn build_utxo_merkle_tree(utxos: &[Utxo]) -> (MerkleTree<MerkleSha256>, FixedBytes<32>) {
    if utxos.is_empty() {
        return (MerkleTree::<MerkleSha256>::new(), FixedBytes::ZERO);
    }

    // Use UTXO IDs directly as leaves (they're already hashes)
    let leaves: Vec<[u8; 32]> = utxos.iter().map(|utxo| utxo.id.0).collect();

    let tree = MerkleTree::<MerkleSha256>::from_leaves(&leaves);
    let root = tree.root().unwrap_or([0u8; 32]);
    (tree, FixedBytes::from_slice(&root))
}

/// Generate a Merkle proof for a UTXO at a given index
pub fn generate_utxo_proof(
    tree: &MerkleTree<MerkleSha256>,
    leaf_index: usize,
) -> Option<Vec<[u8; 32]>> {
    tree.proof(&[leaf_index]).proof_hashes().to_vec().into()
}

/// A fill representing a matched trade
#[derive(Debug, Clone)]
pub struct Fill {
    /// UTXO ID of the maker (older order)
    pub maker_utxo_id: FixedBytes<32>,
    /// UTXO ID of the taker (newer order)
    pub taker_utxo_id: FixedBytes<32>,
    /// Execution price (maker's price)
    pub price: u64,
    /// Quantity of AssetA traded
    pub quantity: u64,
    /// Maker's address
    pub maker: Address,
    /// Taker's address
    pub taker: Address,
    /// Whether maker is selling (true) or buying (false)
    pub maker_is_seller: bool,
}

/// Input to the batch matching process
#[derive(Debug, Clone)]
pub struct BatchInput {
    /// Current batch index (must match on-chain for replay protection)
    pub batch_index: u64,
    /// Expected on-chain UTXO Merkle root (verified via Steel)
    pub utxo_merkle_root: FixedBytes<32>,
    /// Existing UTXOs with their Merkle proofs
    pub existing_utxos_with_proofs: Vec<UtxoWithProof>,
    /// New orders from this batch
    pub new_orders: Vec<Order>,
}

/// Output from the batch matching process (committed to journal)
#[derive(Debug, Clone)]
pub struct BatchOutput {
    /// Batch index (for replay protection)
    pub batch_index: u64,
    /// Fills from matched orders
    pub fills: Vec<Fill>,
    /// New UTXOs (unfilled and partially filled orders)
    pub new_utxos: Vec<Utxo>,
    /// IDs of consumed UTXOs (fully filled)
    pub consumed_utxo_ids: Vec<FixedBytes<32>>,
    /// Merkle root of the new UTXO set
    pub new_utxo_merkle_root: FixedBytes<32>,
}

// Solidity ABI types for encoding/decoding
sol! {
    /// Order struct for Solidity
    struct SolOrder {
        uint8 side; // 0 = Buy, 1 = Sell
        uint64 price;
        uint64 quantity;
        address owner;
        uint64 nonce;
        uint64 expiryBatch;
    }

    /// UTXO struct for Solidity
    struct SolUtxo {
        bytes32 id;
        uint8 side; // 0 = Buy, 1 = Sell
        uint64 price;
        uint64 quantity;
        address owner;
        uint64 nonce;
        uint64 expiryBatch;
    }

    /// Fill struct for Solidity
    struct SolFill {
        bytes32 makerUtxoId;
        bytes32 takerUtxoId;
        uint64 price;
        uint64 quantity;
        address maker;
        address taker;
        bool makerIsSeller;
    }

    /// UTXO with Merkle proof for ABI encoding
    struct SolUtxoWithProof {
        bytes32 id;
        uint8 side;
        uint64 price;
        uint64 quantity;
        address owner;
        uint64 nonce;
        uint64 expiryBatch;
        bytes32[] proofHashes;
        uint256 leafIndex;
    }

    /// Batch input for ABI encoding
    struct SolBatchInput {
        uint64 batchIndex;
        bytes32 utxoMerkleRoot;
        SolUtxoWithProof[] existingUtxosWithProofs;
        SolOrder[] newOrders;
    }

    /// Batch output for Solidity journal decoding
    struct SolBatchOutput {
        uint64 batchIndex;
        SolFill[] fills;
        SolUtxo[] newUtxos;
        bytes32[] consumedUtxoIds;
        bytes32 newUtxoMerkleRoot;
    }

    /// Journal struct that includes Steel commitment and batch output
    /// This is the actual structure committed to the journal and decoded by the contract
    struct SolJournal {
        Steel.Commitment steelCommitment;
        uint64 batchIndex;
        SolFill[] fills;
        SolUtxo[] newUtxos;
        bytes32[] consumedUtxoIds;
        bytes32 newUtxoMerkleRoot;
    }
}

impl From<&Order> for SolOrder {
    fn from(order: &Order) -> Self {
        SolOrder {
            side: order.side as u8,
            price: order.price,
            quantity: order.quantity,
            owner: order.owner,
            nonce: order.nonce,
            expiryBatch: order.expiry_batch,
        }
    }
}

impl From<&SolOrder> for Order {
    fn from(sol: &SolOrder) -> Self {
        Order {
            side: if sol.side == 0 { Side::Buy } else { Side::Sell },
            price: sol.price,
            quantity: sol.quantity,
            owner: sol.owner,
            nonce: sol.nonce,
            expiry_batch: sol.expiryBatch,
        }
    }
}

impl From<&Utxo> for SolUtxo {
    fn from(utxo: &Utxo) -> Self {
        SolUtxo {
            id: utxo.id,
            side: utxo.order.side as u8,
            price: utxo.order.price,
            quantity: utxo.order.quantity,
            owner: utxo.order.owner,
            nonce: utxo.order.nonce,
            expiryBatch: utxo.order.expiry_batch,
        }
    }
}

impl From<&SolUtxo> for Utxo {
    fn from(sol: &SolUtxo) -> Self {
        let order = Order {
            side: if sol.side == 0 { Side::Buy } else { Side::Sell },
            price: sol.price,
            quantity: sol.quantity,
            owner: sol.owner,
            nonce: sol.nonce,
            expiry_batch: sol.expiryBatch,
        };
        Utxo { id: sol.id, order }
    }
}

impl From<&Fill> for SolFill {
    fn from(fill: &Fill) -> Self {
        SolFill {
            makerUtxoId: fill.maker_utxo_id,
            takerUtxoId: fill.taker_utxo_id,
            price: fill.price,
            quantity: fill.quantity,
            maker: fill.maker,
            taker: fill.taker,
            makerIsSeller: fill.maker_is_seller,
        }
    }
}

impl From<&UtxoWithProof> for SolUtxoWithProof {
    fn from(uwp: &UtxoWithProof) -> Self {
        SolUtxoWithProof {
            id: uwp.utxo.id,
            side: uwp.utxo.order.side as u8,
            price: uwp.utxo.order.price,
            quantity: uwp.utxo.order.quantity,
            owner: uwp.utxo.order.owner,
            nonce: uwp.utxo.order.nonce,
            expiryBatch: uwp.utxo.order.expiry_batch,
            proofHashes: uwp
                .proof_hashes
                .iter()
                .map(|h| FixedBytes::from_slice(h))
                .collect(),
            leafIndex: alloy_primitives::U256::from(uwp.leaf_index),
        }
    }
}

impl From<&SolUtxoWithProof> for UtxoWithProof {
    fn from(sol: &SolUtxoWithProof) -> Self {
        let order = Order {
            side: if sol.side == 0 { Side::Buy } else { Side::Sell },
            price: sol.price,
            quantity: sol.quantity,
            owner: sol.owner,
            nonce: sol.nonce,
            expiry_batch: sol.expiryBatch,
        };
        let utxo = Utxo { id: sol.id, order };
        let proof_hashes: Vec<[u8; 32]> = sol
            .proofHashes
            .iter()
            .map(|h| {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(h.as_slice());
                arr
            })
            .collect();
        let leaf_index: usize = sol.leafIndex.try_into().unwrap_or(0);
        UtxoWithProof {
            utxo,
            proof_hashes,
            leaf_index,
        }
    }
}

impl BatchInput {
    /// Convert to Solidity-compatible format for ABI encoding
    pub fn to_sol(&self) -> SolBatchInput {
        SolBatchInput {
            batchIndex: self.batch_index,
            utxoMerkleRoot: self.utxo_merkle_root,
            existingUtxosWithProofs: self
                .existing_utxos_with_proofs
                .iter()
                .map(SolUtxoWithProof::from)
                .collect(),
            newOrders: self.new_orders.iter().map(SolOrder::from).collect(),
        }
    }

    /// Create from Solidity-compatible format (ABI decoding)
    pub fn from_sol(sol: &SolBatchInput) -> Self {
        BatchInput {
            batch_index: sol.batchIndex,
            utxo_merkle_root: sol.utxoMerkleRoot,
            existing_utxos_with_proofs: sol
                .existingUtxosWithProofs
                .iter()
                .map(UtxoWithProof::from)
                .collect(),
            new_orders: sol.newOrders.iter().map(Order::from).collect(),
        }
    }
}

impl BatchOutput {
    /// Convert to Solidity-compatible format for ABI encoding
    pub fn to_sol(&self) -> SolBatchOutput {
        SolBatchOutput {
            batchIndex: self.batch_index,
            fills: self.fills.iter().map(SolFill::from).collect(),
            newUtxos: self.new_utxos.iter().map(SolUtxo::from).collect(),
            consumedUtxoIds: self.consumed_utxo_ids.clone(),
            newUtxoMerkleRoot: self.new_utxo_merkle_root,
        }
    }

    /// Convert to journal format with Steel commitment for on-chain verification
    pub fn to_journal(&self, commitment: Commitment) -> SolJournal {
        SolJournal {
            steelCommitment: commitment,
            batchIndex: self.batch_index,
            fills: self.fills.iter().map(SolFill::from).collect(),
            newUtxos: self.new_utxos.iter().map(SolUtxo::from).collect(),
            consumedUtxoIds: self.consumed_utxo_ids.clone(),
            newUtxoMerkleRoot: self.new_utxo_merkle_root,
        }
    }
}

/// Internal order entry for tracking during matching
#[derive(Clone)]
struct OrderEntry {
    utxo_id: FixedBytes<32>,
    order: Order,
}

/// Main order matching function - runs the limit order book matching algorithm
pub fn match_orders(input: BatchInput) -> BatchOutput {
    use core::cmp::Ordering;

    let current_batch = input.batch_index;

    let mut buy_orders: Vec<OrderEntry> = Vec::new();
    let mut sell_orders: Vec<OrderEntry> = Vec::new();
    let mut consumed_utxo_ids: Vec<FixedBytes<32>> = Vec::new();

    // Total UTXO count for Merkle proof verification (derived from input)
    let utxo_count = input.existing_utxos_with_proofs.len();

    // Process existing UTXOs with proof verification (skip expired ones)
    for utxo_with_proof in input.existing_utxos_with_proofs {
        // Verify UTXO against on-chain Merkle root
        assert!(
            utxo_with_proof.verify(&input.utxo_merkle_root, utxo_count),
            "Invalid Merkle proof for UTXO"
        );

        let utxo = utxo_with_proof.utxo;

        if utxo.is_expired(current_batch) {
            consumed_utxo_ids.push(utxo.id);
            continue;
        }

        let entry = OrderEntry {
            utxo_id: utxo.id,
            order: utxo.order,
        };

        match entry.order.side {
            Side::Buy => buy_orders.push(entry),
            Side::Sell => sell_orders.push(entry),
        }
    }

    // Process new orders (create UTXOs)
    for order in input.new_orders {
        if order.expiry_batch < current_batch {
            continue;
        }

        let utxo = Utxo::new(order);
        let entry = OrderEntry {
            utxo_id: utxo.id,
            order: utxo.order,
        };

        match entry.order.side {
            Side::Buy => buy_orders.push(entry),
            Side::Sell => sell_orders.push(entry),
        }
    }

    // Sort buy orders: price DESC, nonce ASC (price-time priority)
    buy_orders.sort_by(|a, b| match b.order.price.cmp(&a.order.price) {
        Ordering::Equal => a.order.nonce.cmp(&b.order.nonce),
        other => other,
    });

    // Sort sell orders: price ASC, nonce ASC (price-time priority)
    sell_orders.sort_by(|a, b| match a.order.price.cmp(&b.order.price) {
        Ordering::Equal => a.order.nonce.cmp(&b.order.nonce),
        other => other,
    });

    let mut fills: Vec<Fill> = Vec::new();
    let mut buy_idx = 0;
    let mut sell_idx = 0;

    // Match orders while best buy price >= best sell price
    while buy_idx < buy_orders.len() && sell_idx < sell_orders.len() {
        let buy = &buy_orders[buy_idx];
        let sell = &sell_orders[sell_idx];

        if buy.order.price < sell.order.price {
            break;
        }

        // Determine maker (older order by nonce) for price execution
        let (maker, taker, maker_is_seller) = if buy.order.nonce < sell.order.nonce {
            (buy, sell, false)
        } else {
            (sell, buy, true)
        };

        let exec_price = maker.order.price;
        let fill_qty = buy.order.quantity.min(sell.order.quantity);

        let fill = Fill {
            maker_utxo_id: maker.utxo_id,
            taker_utxo_id: taker.utxo_id,
            price: exec_price,
            quantity: fill_qty,
            maker: maker.order.owner,
            taker: taker.order.owner,
            maker_is_seller,
        };
        fills.push(fill);

        let buy_remaining = buy.order.quantity - fill_qty;
        let sell_remaining = sell.order.quantity - fill_qty;

        if buy_remaining == 0 {
            consumed_utxo_ids.push(buy.utxo_id);
            buy_idx += 1;
        } else {
            buy_orders[buy_idx].order.quantity = buy_remaining;
        }

        if sell_remaining == 0 {
            consumed_utxo_ids.push(sell.utxo_id);
            sell_idx += 1;
        } else {
            sell_orders[sell_idx].order.quantity = sell_remaining;
        }
    }

    // Collect remaining orders as new UTXOs
    let mut new_utxos: Vec<Utxo> = Vec::new();

    for entry in buy_orders.into_iter().skip(buy_idx) {
        let utxo = Utxo::new(entry.order);
        new_utxos.push(utxo);
    }

    for entry in sell_orders.into_iter().skip(sell_idx) {
        let utxo = Utxo::new(entry.order);
        new_utxos.push(utxo);
    }

    // Compute new Merkle root from the resulting UTXOs
    let new_utxo_ids: Vec<FixedBytes<32>> = new_utxos.iter().map(|u| u.id).collect();
    let new_utxo_merkle_root = compute_utxo_merkle_root(&new_utxo_ids);

    BatchOutput {
        batch_index: current_batch,
        fills,
        new_utxos,
        consumed_utxo_ids,
        new_utxo_merkle_root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utxo_id_generation() {
        let order = Order {
            side: Side::Buy,
            price: 100,
            quantity: 10,
            owner: Address::ZERO,
            nonce: 1,
            expiry_batch: 100,
        };

        let utxo = Utxo::new(order.clone());
        let expected_id = order.compute_utxo_id();
        assert_eq!(utxo.id, expected_id);
    }

    #[test]
    fn test_utxo_expiry() {
        let order = Order {
            side: Side::Buy,
            price: 100,
            quantity: 10,
            owner: Address::ZERO,
            nonce: 1,
            expiry_batch: 50,
        };

        let utxo = Utxo::new(order);
        assert!(!utxo.is_expired(50));
        assert!(utxo.is_expired(51));
    }

    #[test]
    fn test_merkle_root_computation() {
        let order1 = Order {
            side: Side::Buy,
            price: 100,
            quantity: 10,
            owner: Address::ZERO,
            nonce: 1,
            expiry_batch: 100,
        };
        let order2 = Order {
            side: Side::Sell,
            price: 99,
            quantity: 5,
            owner: Address::ZERO,
            nonce: 2,
            expiry_batch: 100,
        };

        let utxo1 = Utxo::new(order1);
        let utxo2 = Utxo::new(order2);
        let utxos = vec![utxo1.clone(), utxo2.clone()];

        let (tree, root) = build_utxo_merkle_tree(&utxos);

        // Verify root is not zero
        assert_ne!(root, FixedBytes::ZERO);

        // Generate and verify proofs
        let proof1 = generate_utxo_proof(&tree, 0).unwrap();
        let proof2 = generate_utxo_proof(&tree, 1).unwrap();

        let uwp1 = UtxoWithProof {
            utxo: utxo1,
            proof_hashes: proof1,
            leaf_index: 0,
        };
        let uwp2 = UtxoWithProof {
            utxo: utxo2,
            proof_hashes: proof2,
            leaf_index: 1,
        };

        assert!(uwp1.verify(&root, 2));
        assert!(uwp2.verify(&root, 2));
    }

    #[test]
    fn test_merkle_proof_invalid() {
        let order = Order {
            side: Side::Buy,
            price: 100,
            quantity: 10,
            owner: Address::ZERO,
            nonce: 1,
            expiry_batch: 100,
        };

        let utxo = Utxo::new(order);
        let utxos = vec![utxo.clone()];

        let (_tree, _root) = build_utxo_merkle_tree(&utxos);

        // Try with wrong root
        let wrong_root = FixedBytes::from_slice(&[1u8; 32]);
        let uwp = UtxoWithProof {
            utxo,
            proof_hashes: vec![],
            leaf_index: 0,
        };

        assert!(!uwp.verify(&wrong_root, 1));
    }
}
