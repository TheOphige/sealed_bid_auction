# Sealed Bid Auction Contract

A sealed-bid auction implementation using Arbitrum Stylus, written in Rust. In a sealed-bid auction, participants submit hidden bids during the bidding phase. After the bidding deadline, all participants reveal their bids. The highest valid bid wins the auction, and the winner pays their bid amount. The process ensures fairness by preventing bid sniping and maintaining privacy until the reveal phase.

## Features

* **Commit-Reveal Mechanism**: Uses hash commitments to keep bids secret until the reveal phase
* **Supports ERC20 Payments**: Payments are made in ERC20 tokens
* **NFT Auctions**: Designed to auction ERC721 tokens
* **Deterministic Deployment**: Can be deployed via a factory using `CREATE2`
* **Automatic Settlement**: Transfers NFT to the winner and payment to the seller after reveal phase
* **Refund Handling**: Unsuccessful bidders receive refunds
* **Event Logging**: Tracks bids, reveals, and auction settlement

## Quick Start

### Prerequisites

* [Rust](https://rustup.rs/) toolchain
* [Cargo Stylus](https://github.com/OffchainLabs/cargo-stylus)

### Installation

```bash
cargo install cargo-stylus
rustup target add wasm32-unknown-unknown
```

### Build Commands

#### Check contract validity:

```bash
cargo stylus check
```

#### Build for production:

```bash
cargo build --release
```

#### Export ABI:

```bash
cargo stylus export-abi
```

### Deployment

#### Deploy to Arbitrum Sepolia (testnet):

```bash
cargo stylus deploy \
    --endpoint <yourRPCurl> \
    --private-key <yourPrivateKey>
```

## Auction Lifecycle

1. **Initialization**: Seller deploys auction and sets parameters (NFT, token ID, payment token, bidding deadline, reveal deadline)
2. **Bidding Phase**: Bidders submit a commitment hash = keccak256(bidAmount, secretSalt)
3. **Reveal Phase**: Bidders reveal their bid amount and salt, contract verifies commitments
4. **Settlement**: Highest valid bidder wins, NFT is transferred, seller is paid, losers refunded

## Constructor Parameters

```rust
initialize(
    nft_contract: Address,
    token_id: U256,
    payment_token: Address,
    bidding_end: U256,
    reveal_end: U256
) -> Result<(), Vec<u8>>
```

* `nft_contract`: ERC721 NFT contract address
* `token_id`: NFT being auctioned
* `payment_token`: ERC20 token used for payment
* `bidding_end`: Timestamp when bidding ends
* `reveal_end`: Timestamp when reveal phase ends

## Core Functions

#### Commit Bid

```rust
commit_bid(commitment: [u8; 32]) -> Result<(), Vec<u8>>
```

Saves a commitment hash during bidding phase.

#### Reveal Bid

```rust
reveal_bid(bid_amount: U256, salt: [u8; 32]) -> Result<(), Vec<u8>>
```

Reveals the bid during reveal phase. Verifies commitment and updates highest bid.

#### Finalize Auction

```rust
finalize() -> Result<(), Vec<u8>>
```

Finalizes auction after reveal deadline. Transfers NFT to winner, seller receives payment, refunds processed.

## View Functions

```rust
get_highest_bid() -> U256
get_highest_bidder() -> Address
get_commitment(address: Address) -> [u8; 32]
has_ended() -> bool
```

## Events

* `BidCommitted(address indexed bidder, bytes32 commitment)`
* `BidRevealed(address indexed bidder, uint256 amount)`
* `AuctionFinalized(address indexed winner, uint256 amount)`

## Security Features

* **Commit-Reveal**: Prevents sniping and ensures fairness
* **Time Windows**: Strict enforcement of bidding and reveal deadlines
* **Access Control**: Only seller can finalize auction
* **Refund Safety**: Ensures losing bidders get refunds
* **Input Validation**: Validates bid amounts and reveal commitments

## Factory Integration

This contract is designed to work with the `SealedBidAuctionFactory`:

1. Factory embeds compiled Wasm bytecode
2. Factory deploys new auction instances using `CREATE2`
3. Each auction instance operates independently

## Development

### Run Tests

```bash
cargo test
```

### Local Development

```bash
cargo stylus check --endpoint http://localhost:8547
```

## License

This project is licensed under MIT OR Apache-2.0.
