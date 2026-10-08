use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env, String,
};

struct Fixture {
    env: Env,
    client: DonationsClient<'static>,
    admin: Address,
    charity: Address,
    donor: Address,
    token: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let admin = Address::generate(&env);
    let charity = Address::generate(&env);
    let donor = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&donor, &1_000_000);

    let id = env.register(Donations, (admin.clone(),));
    let client = DonationsClient::new(&env, &id);

    Fixture {
        env,
        client,
        admin,
        charity,
        donor,
        token,
    }
}

fn open_campaign(f: &Fixture) -> u32 {
    f.client.create_campaign(
        &f.admin,
        &f.charity,
        &f.token,
        &String::from_str(&f.env, "Clean water for Kano"),
        &String::from_str(&f.env, "Boreholes for three rural schools"),
        &500_000,
        &10_000,
    )
}

#[test]
fn constructor_sets_admin() {
    let f = setup();
    assert_eq!(f.client.admin_address(), f.admin);
    assert!(!f.client.is_paused());
}

#[test]
fn create_campaign_assigns_sequential_ids() {
    let f = setup();
    assert_eq!(open_campaign(&f), 1);
    assert_eq!(open_campaign(&f), 2);
    assert_eq!(f.client.campaign_count(), 2);
    let c = f.client.get_campaign(&1);
    assert_eq!(c.beneficiary, f.charity);
    assert_eq!(c.raised, 0);
    assert!(c.open);
}

#[test]
fn create_campaign_validates_input() {
    let f = setup();
    let title = String::from_str(&f.env, "Food bank");
    let desc = String::from_str(&f.env, "");
    assert_eq!(
        f.client
            .try_create_campaign(&f.admin, &f.charity, &f.token, &title, &desc, &0, &10_000),
        Err(Ok(DonationError::InvalidGoal))
    );
    assert_eq!(
        f.client
            .try_create_campaign(&f.admin, &f.charity, &f.token, &title, &desc, &10, &999),
        Err(Ok(DonationError::InvalidDeadline))
    );
    let empty = String::from_str(&f.env, "");
    assert_eq!(
        f.client
            .try_create_campaign(&f.admin, &f.charity, &f.token, &empty, &desc, &10, &10_000),
        Err(Ok(DonationError::TextTooLong))
    );
}

#[test]
fn donate_sends_funds_directly_to_beneficiary() {
    let f = setup();
    let id = open_campaign(&f);
    let memo = String::from_str(&f.env, "in memory of grandma");
    let receipt = f.client.donate(&f.donor, &id, &25_000, &memo);
    assert_eq!(receipt, 1);

    let token = TokenClient::new(&f.env, &f.token);
    assert_eq!(token.balance(&f.charity), 25_000);
    assert_eq!(token.balance(&f.donor), 975_000);
    assert_eq!(token.balance(&f.client.address), 0);

    let c = f.client.get_campaign(&id);
    assert_eq!(c.raised, 25_000);
    assert_eq!(c.donor_count, 1);
    assert_eq!(c.donation_count, 1);

    let r = f.client.get_receipt(&receipt);
    assert_eq!(r.donor, f.donor);
    assert_eq!(r.amount, 25_000);
    assert_eq!(r.memo, memo);
}

#[test]
fn repeat_donor_counted_once() {
    let f = setup();
    let id = open_campaign(&f);
    let memo = String::from_str(&f.env, "");
    f.client.donate(&f.donor, &id, &100, &memo);
    f.client.donate(&f.donor, &id, &200, &memo);
    let c = f.client.get_campaign(&id);
    assert_eq!(c.donor_count, 1);
    assert_eq!(c.donation_count, 2);
    assert_eq!(f.client.donor_total(&id, &f.donor), 300);
    assert_eq!(f.client.receipt_count(), 2);
}

#[test]
fn donate_rejects_bad_requests() {
    let f = setup();
    let id = open_campaign(&f);
    let memo = String::from_str(&f.env, "");
    assert_eq!(
        f.client.try_donate(&f.donor, &id, &0, &memo),
        Err(Ok(DonationError::InvalidAmount))
    );
    assert_eq!(
        f.client.try_donate(&f.donor, &99, &10, &memo),
        Err(Ok(DonationError::CampaignNotFound))
    );
    assert_eq!(
        f.client.try_donate(&f.charity, &id, &10, &memo),
        Err(Ok(DonationError::SelfDonation))
    );
}

#[test]
fn donate_after_deadline_fails() {
    let f = setup();
    let id = open_campaign(&f);
    f.env.ledger().set_timestamp(10_001);
    assert_eq!(
        f.client
            .try_donate(&f.donor, &id, &10, &String::from_str(&f.env, "")),
        Err(Ok(DonationError::CampaignExpired))
    );
}

#[test]
fn closed_campaign_rejects_donations() {
    let f = setup();
    let id = open_campaign(&f);
    f.client.close_campaign(&f.admin, &id);
    assert!(!f.client.get_campaign(&id).open);
    assert_eq!(
        f.client
            .try_donate(&f.donor, &id, &10, &String::from_str(&f.env, "")),
        Err(Ok(DonationError::CampaignClosed))
    );
    assert_eq!(
        f.client.try_close_campaign(&f.admin, &id),
        Err(Ok(DonationError::CampaignClosed))
    );
}

#[test]
fn stranger_cannot_close_campaign() {
    let f = setup();
    let id = open_campaign(&f);
    let stranger = Address::generate(&f.env);
    assert_eq!(
        f.client.try_close_campaign(&stranger, &id),
        Err(Ok(DonationError::Unauthorized))
    );
}

#[test]
fn pause_blocks_donations() {
    let f = setup();
    let id = open_campaign(&f);
    f.client.set_paused(&true);
    assert!(f.client.is_paused());
    assert_eq!(
        f.client
            .try_donate(&f.donor, &id, &10, &String::from_str(&f.env, "")),
        Err(Ok(DonationError::Paused))
    );
}
