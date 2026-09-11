#![cfg(test)]

//! Shared test fixture for router tests: local network of one router, one
//! test adapter, three SEP-41 tokens and two pools, all wired and funded
//! per-test by the callers.

use super::{Router, RouterClient};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};
use stellariq_interfaces::SwapStep;
use stellariq_test_adapter::{TestAdapter, TestAdapterClient};

pub const DEADLINE: u64 = 1_000_000;

pub struct Fixture {
    pub env: Env,
    pub admin: Address,
    pub trader: Address,
    pub router_id: Address,
    pub adapter_id: Address,
    pub token_a: Address,
    pub token_b: Address,
    pub token_c: Address,
    pub pool: Address,
    pub pool2: Address,
}

pub fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1000);

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let router_id = env.register(Router, ());
    let adapter_id = env.register(TestAdapter, ());
    let pool = Address::generate(&env);
    let pool2 = Address::generate(&env);

    let token_a = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_c = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let client = RouterClient::new(&env, &router_id);
    client.initialize(&admin);

    let adapter_client = TestAdapterClient::new(&env, &adapter_id);
    adapter_client.initialize(&admin);
    client.set_protocol(&symbol_short!("test"), &adapter_id);

    Fixture {
        env,
        admin,
        trader,
        router_id,
        adapter_id,
        token_a,
        token_b,
        token_c,
        pool,
        pool2,
    }
}

pub fn router(f: &Fixture) -> RouterClient<'_> {
    RouterClient::new(&f.env, &f.router_id)
}

pub fn adapter(f: &Fixture) -> TestAdapterClient<'_> {
    TestAdapterClient::new(&f.env, &f.adapter_id)
}

pub fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    StellarAssetClient::new(env, token).mint(to, &amount);
}

pub fn balance(env: &Env, token: &Address, holder: &Address) -> i128 {
    TokenClient::new(env, token).balance(holder)
}

/// Fund a single-hop A->B scenario: trader holds A, adapter holds B.
pub fn fund_single_hop(f: &Fixture, trader_a: i128, adapter_b: i128) {
    mint(&f.env, &f.token_a, &f.trader, trader_a);
    mint(&f.env, &f.token_b, &f.adapter_id, adapter_b);
}

pub fn step(f: &Fixture, token_in: &Address, token_out: &Address, min: i128) -> SwapStep {
    step_on_pool(&f.pool, token_in, token_out, min)
}

pub fn step_on_pool(
    pool: &Address,
    token_in: &Address,
    token_out: &Address,
    min: i128,
) -> SwapStep {
    SwapStep {
        protocol: symbol_short!("test"),
        pool: pool.clone(),
        token_in: token_in.clone(),
        token_out: token_out.clone(),
        amount_out_min: min,
    }
}
