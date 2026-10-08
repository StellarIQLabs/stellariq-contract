#![no_std]

//! StellarIQ Give - transparent charity donations on Stellar.
//!
//! Charities (or organizers acting for them) open campaigns with a goal, a
//! deadline, a beneficiary wallet and the SEP-41 token they accept. Donors
//! give through `donate`, which moves funds straight from the donor to the
//! beneficiary in the same transaction and records an on-chain receipt.
//!
//! Design choices:
//! - Non-custodial: the contract never holds donated funds, so there is no
//!   withdraw path to attack and nothing to drain.
//! - Every donation emits a `donation_made` event and stores a receipt that
//!   anyone can look up by id, which is what the app and indexer read.
//! - Donating any other asset is handled off-contract by the StellarIQ swap
//!   router: the app converts first, then calls `donate` with the campaign
//!   token, in one transaction.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token::TokenClient,
    Address, Env, String,
};

#[cfg(test)]
mod test;

pub const EVENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_TITLE_LEN: u32 = 80;
pub const MAX_TEXT_LEN: u32 = 280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum DonationError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
    InvalidAmount = 5,
    InvalidGoal = 6,
    InvalidDeadline = 7,
    CampaignNotFound = 8,
    CampaignClosed = 9,
    CampaignExpired = 10,
    TextTooLong = 11,
    ReceiptNotFound = 12,
    SelfDonation = 13,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Campaign {
    pub id: u32,
    pub creator: Address,
    pub beneficiary: Address,
    pub token: Address,
    pub title: String,
    pub description: String,
    /// Target amount in the token's smallest unit.
    pub goal: i128,
    /// Unix timestamp (seconds) after which donations are rejected.
    pub deadline: u64,
    pub raised: i128,
    pub donor_count: u32,
    pub donation_count: u32,
    pub open: bool,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub id: u64,
    pub campaign_id: u32,
    pub donor: Address,
    pub amount: i128,
    pub memo: String,
    pub timestamp: u64,
    pub ledger: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignCreated {
    #[topic]
    pub campaign_id: u32,
    pub creator: Address,
    pub beneficiary: Address,
    pub token: Address,
    pub goal: i128,
    pub deadline: u64,
    pub version: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DonationMade {
    #[topic]
    pub campaign_id: u32,
    #[topic]
    pub donor: Address,
    pub receipt_id: u64,
    pub amount: i128,
    pub raised: i128,
    pub version: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignClosed {
    #[topic]
    pub campaign_id: u32,
    pub raised: i128,
    pub version: u32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Paused,
    CampaignCount,
    ReceiptCount,
    Campaign(u32),
    Receipt(u64),
    DonorTotal(u32, Address),
}

const TTL_THRESHOLD: u32 = 17_280;
const TTL_EXTEND: u32 = 518_400;

#[contract]
pub struct Donations;

#[contractimpl]
impl Donations {
    // -- Lifecycle --------------------------------------------------------

    /// Runs once at deploy time, so the admin can never be front-run.
    pub fn __constructor(env: Env, admin: Address) {
        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::Paused, &false);
        s.set(&DataKey::CampaignCount, &0u32);
        s.set(&DataKey::ReceiptCount, &0u64);
        s.extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
    }

    pub fn admin_address(env: Env) -> Result<Address, DonationError> {
        Self::admin(&env)
    }

    /// Emergency stop for new campaigns and donations. Admin only.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), DonationError> {
        Self::admin(&env)?.require_auth();
        env.storage().instance().set(&DataKey::Paused, &paused);
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    // -- Campaigns --------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn create_campaign(
        env: Env,
        creator: Address,
        beneficiary: Address,
        token: Address,
        title: String,
        description: String,
        goal: i128,
        deadline: u64,
    ) -> Result<u32, DonationError> {
        Self::ensure_live(&env)?;
        creator.require_auth();
        if goal <= 0 {
            return Err(DonationError::InvalidGoal);
        }
        let now = env.ledger().timestamp();
        if deadline <= now {
            return Err(DonationError::InvalidDeadline);
        }
        if title.is_empty() || title.len() > MAX_TITLE_LEN || description.len() > MAX_TEXT_LEN {
            return Err(DonationError::TextTooLong);
        }

        let id: u32 = Self::campaign_count(env.clone()) + 1;
        let campaign = Campaign {
            id,
            creator: creator.clone(),
            beneficiary: beneficiary.clone(),
            token: token.clone(),
            title,
            description,
            goal,
            deadline,
            raised: 0,
            donor_count: 0,
            donation_count: 0,
            open: true,
            created_at: now,
        };
        Self::put_campaign(&env, &campaign);
        env.storage().instance().set(&DataKey::CampaignCount, &id);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND);

        CampaignCreated {
            campaign_id: id,
            creator,
            beneficiary,
            token,
            goal,
            deadline,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(id)
    }

    /// Stop accepting donations. The campaign creator or the admin may close.
    pub fn close_campaign(env: Env, caller: Address, id: u32) -> Result<(), DonationError> {
        caller.require_auth();
        let mut c = Self::get_campaign(env.clone(), id)?;
        if caller != c.creator && caller != Self::admin(&env)? {
            return Err(DonationError::Unauthorized);
        }
        if !c.open {
            return Err(DonationError::CampaignClosed);
        }
        c.open = false;
        Self::put_campaign(&env, &c);
        CampaignClosed {
            campaign_id: id,
            raised: c.raised,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    pub fn get_campaign(env: Env, id: u32) -> Result<Campaign, DonationError> {
        let key = DataKey::Campaign(id);
        let c: Campaign = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(DonationError::CampaignNotFound)?;
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND);
        Ok(c)
    }

    pub fn campaign_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CampaignCount)
            .unwrap_or(0)
    }

    // -- Donations --------------------------------------------------------

    /// Give `amount` of the campaign token. Funds go directly from `donor`
    /// to the campaign beneficiary; the contract keeps only the receipt.
    pub fn donate(
        env: Env,
        donor: Address,
        campaign_id: u32,
        amount: i128,
        memo: String,
    ) -> Result<u64, DonationError> {
        Self::ensure_live(&env)?;
        donor.require_auth();
        if amount <= 0 {
            return Err(DonationError::InvalidAmount);
        }
        if memo.len() > MAX_TEXT_LEN {
            return Err(DonationError::TextTooLong);
        }
        let mut c = Self::get_campaign(env.clone(), campaign_id)?;
        if !c.open {
            return Err(DonationError::CampaignClosed);
        }
        if env.ledger().timestamp() > c.deadline {
            return Err(DonationError::CampaignExpired);
        }
        if donor == c.beneficiary {
            return Err(DonationError::SelfDonation);
        }

        TokenClient::new(&env, &c.token).transfer(&donor, &c.beneficiary, &amount);

        let total_key = DataKey::DonorTotal(campaign_id, donor.clone());
        let prev: i128 = env.storage().persistent().get(&total_key).unwrap_or(0);
        if prev == 0 {
            c.donor_count += 1;
        }
        env.storage().persistent().set(&total_key, &(prev + amount));
        env.storage()
            .persistent()
            .extend_ttl(&total_key, TTL_THRESHOLD, TTL_EXTEND);

        c.raised = c
            .raised
            .checked_add(amount)
            .ok_or(DonationError::InvalidAmount)?;
        c.donation_count += 1;
        Self::put_campaign(&env, &c);

        let receipt_id: u64 = Self::receipt_count(env.clone()) + 1;
        let receipt = Receipt {
            id: receipt_id,
            campaign_id,
            donor: donor.clone(),
            amount,
            memo,
            timestamp: env.ledger().timestamp(),
            ledger: env.ledger().sequence(),
        };
        let rkey = DataKey::Receipt(receipt_id);
        env.storage().persistent().set(&rkey, &receipt);
        env.storage()
            .persistent()
            .extend_ttl(&rkey, TTL_THRESHOLD, TTL_EXTEND);
        env.storage()
            .instance()
            .set(&DataKey::ReceiptCount, &receipt_id);

        DonationMade {
            campaign_id,
            donor,
            receipt_id,
            amount,
            raised: c.raised,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(receipt_id)
    }

    pub fn get_receipt(env: Env, id: u64) -> Result<Receipt, DonationError> {
        env.storage()
            .persistent()
            .get(&DataKey::Receipt(id))
            .ok_or(DonationError::ReceiptNotFound)
    }

    pub fn receipt_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ReceiptCount)
            .unwrap_or(0)
    }

    pub fn donor_total(env: Env, campaign_id: u32, donor: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::DonorTotal(campaign_id, donor))
            .unwrap_or(0)
    }

    // -- Internals --------------------------------------------------------

    fn admin(env: &Env) -> Result<Address, DonationError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(DonationError::NotInitialized)
    }

    fn ensure_live(env: &Env) -> Result<(), DonationError> {
        Self::admin(env)?;
        if Self::is_paused(env.clone()) {
            return Err(DonationError::Paused);
        }
        Ok(())
    }

    fn put_campaign(env: &Env, c: &Campaign) {
        let key = DataKey::Campaign(c.id);
        env.storage().persistent().set(&key, c);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND);
    }
}
