#![cfg_attr(not(feature = "export-abi"), no_main)]
extern crate alloc;

use alloc::vec::Vec;
use stylus_sdk::{
    alloy_primitives::{Address, B256, U256},
    alloy_sol_types::sol,
    block, call, contract, crypto, msg,
    prelude::*,
};

// ERC721 interface for NFT transfers
sol_interface! {
    interface IERC721 {
        function transferFrom(address from, address to, uint256 tokenId) external;
        function ownerOf(uint256 tokenId) external view returns (address);
        function getApproved(uint256 tokenId) external view returns (address);
        function isApprovedForAll(address owner, address operator) external view returns (bool);
    }
}

// Custom errors
sol! {
    error NotOwner();
    error AuctionNotActive();
    error AuctionAlreadyFinalized();
    error InvalidDuration();
    error ZeroAddress();
    error NotApproved();
    error NotNFTOwner();
    error InvalidCommit();
    error RevealNotOpen();
    error CommitPhaseOver();
    error NoDeposit();
    error PaymentFailed();
    error NFTTransferFailed();
    error OnlySeller();
    error AlreadyCommitted();
    error AlreadyRevealed();
    error AuctionNotEnded();
    error NothingToWithdraw();
}

#[derive(SolidityError)]
pub enum SealedBidError {
    NotOwner(NotOwner),
    AuctionNotActive(AuctionNotActive),
    AuctionAlreadyFinalized(AuctionAlreadyFinalized),
    InvalidDuration(InvalidDuration),
    ZeroAddress(ZeroAddress),
    NotApproved(NotApproved),
    NotNFTOwner(NotNFTOwner),
    InvalidCommit(InvalidCommit),
    RevealNotOpen(RevealNotOpen),
    CommitPhaseOver(CommitPhaseOver),
    NoDeposit(NoDeposit),
    PaymentFailed(PaymentFailed),
    NFTTransferFailed(NFTTransferFailed),
    OnlySeller(OnlySeller),
    AlreadyCommitted(AlreadyCommitted),
    AlreadyRevealed(AlreadyRevealed),
    AuctionNotEnded(AuctionNotEnded),
    NothingToWithdraw(NothingToWithdraw),
}

// Storage
sol_storage! {
    #[entrypoint]
    pub struct SealedBidAuction {
        // basic auction metadata
        address seller;
        address nft_contract;
        uint256 token_id;

        // economic params
        uint256 reserve_price;   // min acceptable winning bid
        uint256 min_deposit;     // deposit required to commit

        // timelines (unix seconds)
        uint256 start_time;
        uint256 commit_end;      // end timestamp of commit phase
        uint256 reveal_end;      // end timestamp of reveal phase

        // state
        bool finalized;
        address highest_bidder;
        uint256 highest_bid;

        // mappings
        mapping(address => bytes32) commitments; // commit hash => saved
        mapping(address => uint256) deposits;    // total deposit posted by address
        mapping(address => bool) revealed;       // whether address already revealed
        mapping(address => uint256) refunds;     // withdrawnable refunds
    }
}

#[public]
impl SealedBidAuction {
    /// Initialize auction. Called once after deployment.
    pub fn new(
        &mut self,
        seller: Address,
        nft_contract: Address,
        token_id: U256,
        reserve_price: U256,
        commit_duration: U256,
        reveal_duration: U256,
        min_deposit: U256,
    ) -> Result<(), SealedBidError> {
        if seller == Address::ZERO || nft_contract == Address::ZERO {
            return Err(SealedBidError::ZeroAddress(ZeroAddress {}));
        }

        if commit_duration == U256::ZERO || reveal_duration == U256::ZERO {
            return Err(SealedBidError::InvalidDuration(InvalidDuration {}));
        }

        if min_deposit == U256::ZERO {
            return Err(SealedBidError::InvalidCommit(InvalidCommit {}));
        }

        // set state
        self.seller.set(seller);
        self.nft_contract.set(nft_contract);
        self.token_id.set(token_id);
        self.reserve_price.set(reserve_price);
        self.min_deposit.set(min_deposit);

        let now = U256::from(block::timestamp());
        self.start_time.set(now);
        self.commit_end.set(now + commit_duration);
        self.reveal_end.set(now + commit_duration + reveal_duration);

        self.finalized.set(false);
        self.highest_bidder.set(Address::ZERO);
        self.highest_bid.set(U256::ZERO);

        // Verify NFT ownership and approval
        self.verify_nft_authorization(seller)?;

        Ok(())
    }

    /// Commit a bid hash (keccak256(abi.encodePacked(bid, nonce))).
    /// Must send at least `min_deposit` as msg.value. Multiple commits from same address add deposits,
    /// but only the last commitment is considered (so discourage multiple commits).
    pub fn commit(&mut self, commitment: B256) -> Result<(), SealedBidError> {
        let now = U256::from(block::timestamp());
        if now >= self.commit_end.get() {
            return Err(SealedBidError::CommitPhaseOver(CommitPhaseOver {}));
        }

        let sender = msg::sender();
        if commitment == B256::ZERO {
            return Err(SealedBidError::InvalidCommit(InvalidCommit {}));
        }

        let value = msg::value();
        if value < self.min_deposit.get() && self.deposits.get(sender) == U256::ZERO {
            // If the caller hasn't deposited before, require at least min_deposit
            return Err(SealedBidError::NoDeposit(NoDeposit {}));
        }

        // store/overwrite commitment
        self.commitments.setter(sender).set(commitment);

        // accumulate deposits
        if value > U256::ZERO {
            let prev = self.deposits.get(sender);
            self.deposits.setter(sender).set(prev + value);
        }

        Ok(())
    }

    /// Reveal a previously committed bid.
    /// `bid` must match the committed hash when combined with `nonce`:
    /// keccak256(bid || nonce) == commitment
    pub fn reveal(&mut self, bid: U256, nonce: U256) -> Result<(), SealedBidError> {
        let now = U256::from(block::timestamp());
        if now <= self.commit_end.get() {
            return Err(SealedBidError::RevealNotOpen(RevealNotOpen {}));
        }
        if now >= self.reveal_end.get() {
            return Err(SealedBidError::AuctionNotEnded(AuctionNotEnded {}));
        }

        let sender = msg::sender();

        if self.revealed.get(sender) {
            return Err(SealedBidError::AlreadyRevealed(AlreadyRevealed {}));
        }

        let commitment = self.commitments.get(sender);
        if commitment == B256::ZERO {
            return Err(SealedBidError::InvalidCommit(InvalidCommit {}));
        }

        // Recompute keccak256(bid || nonce) and compare
        let mut preimage: Vec<u8> = Vec::new();
        preimage.extend_from_slice(&bid.as_le_bytes());
        preimage.extend_from_slice(&nonce.as_le_bytes());
        let computed = B256::from_slice(&crypto::keccak(preimage)[0..32]);

        if computed != commitment {
            // invalid reveal: mark revealed so attacker cannot retry; deposit is forfeited
            self.revealed.setter(sender).set(true);
            // deposit remains in contract (forfeited)
            return Err(SealedBidError::InvalidCommit(InvalidCommit {}));
        }

        // valid reveal
        self.revealed.setter(sender).set(true);

        // get deposit for this sender
        let depos = self.deposits.get(sender);

        if depos < self.min_deposit.get() {
            // insufficient deposit -> treat as invalid (forfeit)
            return Err(SealedBidError::NoDeposit(NoDeposit {}));
        }

        // Accept the revealed bid only if bid is greater than current highest.
        if bid > self.highest_bid.get() {
            // previous highest becomes refundable (its deposit + bid is refunded to previous highest bidder)
            let prev_high = self.highest_bidder.get();
            if prev_high != Address::ZERO {
                // give previous bidder a withdrawable refund equal to their deposit + previous bid
                // (we assume previous bid amount was not yet kept by seller)
                let mut prev_ref = self.refunds.get(prev_high);
                // Add previous bid amount + deposit previously held by the previous highest bidder.
                // We don't store previous bidder's deposit separately here, so assume deposit tracked in deposits map.
                let prev_deposit = self.deposits.get(prev_high);
                prev_ref = prev_ref + self.highest_bid.get() + prev_deposit;
                self.refunds.setter(prev_high).set(prev_ref);
            }

            // set new highest (and keep this bidder's deposit in contract until finalize or refund)
            self.highest_bid.set(bid);
            self.highest_bidder.set(sender);

            // For the current revealer, we reduce their deposit by nothing now; funds stay locked
            // actual funds transfer to seller happens in finalize
        } else {
            // Not a winning bid — allow withdraw later (bid + deposit). We'll store refund now.
            let mut r = self.refunds.get(sender);
            r = r + bid + depos;
            self.refunds.setter(sender).set(r);
        }

        Ok(())
    }

    /// Finalize auction after reveal period. Transfers NFT to winner (if reserve met),
    /// sends payments to seller, and unlocks refunds.
    pub fn finalize(&mut self) -> Result<(), SealedBidError> {
        let now = U256::from(block::timestamp());
        if now < self.reveal_end.get() {
            return Err(SealedBidError::AuctionNotEnded(AuctionNotEnded {}));
        }
        if self.finalized.get() {
            return Err(SealedBidError::AuctionAlreadyFinalized(AuctionAlreadyFinalized {}));
        }

        let seller = self.seller.get();
        let winner = self.highest_bidder.get();
        let winning_bid = self.highest_bid.get();
        let reserve = self.reserve_price.get();

        // If there is a valid highest bid meeting reserve, settle
        if winner != Address::ZERO && winning_bid >= reserve {
            // Transfer NFT from seller -> winner
            self.transfer_nft(seller, winner)?;

            // Compute amount to send to seller: winning_bid
            if winning_bid > U256::ZERO {
                self.transfer_payment(seller, winning_bid)?;
            }

            // The auction contract may still hold deposits: give bidders ability to withdraw their refunds
            // For the winner, any deposit they posted is refundable minus policy; here we choose to refund deposit.
            let winner_deposit = self.deposits.get(winner);
            if winner_deposit > U256::ZERO {
                let prev = self.refunds.get(winner);
                self.refunds.setter(winner).set(prev + winner_deposit);
            }
        } else {
            // No valid winning bid: seller can reclaim the NFT (it remains with seller until transfer).
            // Nothing to transfer. Optionally mark refunds for all revealers: everyone can withdraw their deposits + bids recorded.
            // We will not iterate over bidders (no dynamic list). Deposits are withdrawable by callers via withdraw_refund().
        }

        self.finalized.set(true);
        Ok(())
    }

    /// Withdraw refunds (bid + deposit) available to caller.
    pub fn withdraw_refund(&mut self) -> Result<(), SealedBidError> {
        let caller = msg::sender();
        let amount = self.refunds.get(caller);
        if amount == U256::ZERO {
            return Err(SealedBidError::NothingToWithdraw(NothingToWithdraw {}));
        }

        // zero out before transfer (checks-effects-interactions)
        self.refunds.setter(caller).set(U256::ZERO);

        let result = call::transfer_eth(caller, amount);
        if result.is_err() {
            // restore on failure
            let prev = self.refunds.get(caller);
            self.refunds.setter(caller).set(prev + amount);
            return Err(SealedBidError::PaymentFailed(PaymentFailed {}));
        }

        Ok(())
    }

    /// Allow seller to stop auction early (only if not finalized)
    pub fn cancel_auction(&mut self) -> Result<(), SealedBidError> {
        if msg::sender() != self.seller.get() {
            return Err(SealedBidError::OnlySeller(OnlySeller {}));
        }
        if self.finalized.get() {
            return Err(SealedBidError::AuctionAlreadyFinalized(AuctionAlreadyFinalized {}));
        }

        // Mark finalized so no further actions expected; refunds can be withdrawn by callers
        self.finalized.set(true);
        Ok(())
    }

    /// Helper views
    pub fn get_details(&self) -> (Address, Address, U256, U256, U256, U256, U256, bool, Address, U256) {
        (
            self.seller.get(),
            self.nft_contract.get(),
            self.token_id.get(),
            self.reserve_price.get(),
            self.min_deposit.get(),
            self.commit_end.get(),
            self.reveal_end.get(),
            self.finalized.get(),
            self.highest_bidder.get(),
            self.highest_bid.get(),
        )
    }

    pub fn seller(&self) -> Address {
        self.seller.get()
    }
    pub fn nft_contract(&self) -> Address {
        self.nft_contract.get()
    }
    pub fn token_id(&self) -> U256 {
        self.token_id.get()
    }
    pub fn reserve_price(&self) -> U256 {
        self.reserve_price.get()
    }
    pub fn min_deposit(&self) -> U256 {
        self.min_deposit.get()
    }
    pub fn commit_end(&self) -> U256 {
        self.commit_end.get()
    }
    pub fn reveal_end(&self) -> U256 {
        self.reveal_end.get()
    }
    pub fn finalized(&self) -> bool {
        self.finalized.get()
    }
    pub fn highest_bidder(&self) -> Address {
        self.highest_bidder.get()
    }
    pub fn highest_bid(&self) -> U256 {
        self.highest_bid.get()
    }

    /// Allow caller to check their refundable amount
    pub fn refund_of(&self, who: Address) -> U256 {
        self.refunds.get(who)
    }
}

impl SealedBidAuction {
    /// Verifies seller owns NFT and contract is approved to transfer it
    fn verify_nft_authorization(&mut self, seller: Address) -> Result<(), SealedBidError> {
        let nft_contract = IERC721::new(self.nft_contract.get());
        let token_id = self.token_id.get();

        // owner_of
        let owner_res = nft_contract.owner_of(call::Call::new_in(self), token_id);
        match owner_res {
            Ok(owner) => {
                if owner != seller {
                    return Err(SealedBidError::NotNFTOwner(NotNFTOwner {}));
                }
            }
            Err(_) => return Err(SealedBidError::NFTTransferFailed(NFTTransferFailed {})),
        }

        let contract_address = contract::address();
        let approved_res = nft_contract.get_approved(call::Call::new_in(self), token_id);
        let approved_for_all_res = nft_contract.is_approved_for_all(call::Call::new_in(self), seller, contract_address);

        let is_approved = match approved_res {
            Ok(approved) => approved == contract_address,
            Err(_) => false,
        };

        let is_approved_for_all = match approved_for_all_res {
            Ok(ap) => ap,
            Err(_) => false,
        };

        if !is_approved && !is_approved_for_all {
            return Err(SealedBidError::NotApproved(NotApproved {}));
        }

        Ok(())
    }

    /// Transfer NFT with safety check
    fn transfer_nft(&mut self, from: Address, to: Address) -> Result<(), SealedBidError> {
        let nft_contract = IERC721::new(self.nft_contract.get());
        let token_id = self.token_id.get();
        let res = nft_contract.transfer_from(call::Call::new_in(self), from, to, token_id);
        if res.is_err() {
            return Err(SealedBidError::NFTTransferFailed(NFTTransferFailed {}));
        }
        Ok(())
    }

    /// Transfer payment (ETH) to `to`
    fn transfer_payment(&self, to: Address, amount: U256) -> Result<(), SealedBidError> {
        if to == Address::ZERO {
            return Err(SealedBidError::ZeroAddress(ZeroAddress {}));
        }
        if amount == U256::ZERO {
            return Err(SealedBidError::PaymentFailed(PaymentFailed {}));
        }
        let res = call::transfer_eth(to, amount);
        if res.is_err() {
            return Err(SealedBidError::PaymentFailed(PaymentFailed {}));
        }
        Ok(())
    }
}
