#![cfg(test)]

//! Residual coverage: trader-authorization negatives (selective auth mocks),
//! malformed inputs, boundary values, large values, and adapter admin gating.
//! Together with `test.rs` (lifecycle/validation) and `test_aggregation.rs`
//! (execution), this completes the Task 8 matrix.

use super::fixture::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    vec as sorovec, Address, Env, IntoVal,
};
use stellariq_interfaces::RouterError;
use stellariq_test_adapter::TestAdapter;

// -- Trader authorization ------------------------------------------------------
// Setup WITHOUT blanket mocks: only admin-authorized management calls are
// mocked, so the trader's missing authorization must fail at the host level
// (outer Err), never as a contract-level RouterError.

fn setup_selective_auth() -> Fixture {
    let env = Env::default();
    env.ledger().set_timestamp(1000);

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let router_id = env.register(super::Router, ());
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

    // initialize() calls need no auth.
    super::RouterClient::new(&env, &router_id).initialize(&admin);
    stellariq_test_adapter::TestAdapterClient::new(&env, &adapter_id).initialize(&admin);

    // Mock ONLY admin management + funding. Anything requiring the trader
    // (or anyone else) stays unauthorized.
    env.mock_auths(&[
        MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &router_id,
                fn_name: "set_protocol",
                args: (symbol_short!("test"), adapter_id.clone()).into_val(&env),
                sub_invokes: &[],
            },
        },
        MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &token_a,
                fn_name: "mint",
                args: (trader.clone(), 10_000i128).into_val(&env),
                sub_invokes: &[],
            },
        },
        MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &token_b,
                fn_name: "mint",
                args: (adapter_id.clone(), 10_000i128).into_val(&env),
                sub_invokes: &[],
            },
        },
    ]);
    super::RouterClient::new(&env, &router_id).set_protocol(&symbol_short!("test"), &adapter_id);
    mint(&env, &token_a, &trader, 10_000);
    mint(&env, &token_b, &adapter_id, 10_000);

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

#[test]
fn swap_without_trader_auth_fails_at_host_level() {
    let f = setup_selective_auth();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert!(res.is_err());
    // Must NOT be a contract-level error: auth never got that far.
    assert!(!matches!(res, Err(Ok(_))));
    // And no funds moved.
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 0);
}

#[test]
fn swap_with_wrong_trader_auth_fails() {
    // Trader authorizes nothing; an impostor's signature on identical args
    // cannot move the trader's funds (args bind the trader address).
    let f = setup_selective_auth();
    let impostor = Address::generate(&f.env);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    f.env.mock_auths(&[MockAuth {
        address: &impostor,
        invoke: &MockAuthInvoke {
            contract: &f.router_id,
            fn_name: "swap",
            args: (
                f.trader.clone(),
                f.token_a.clone(),
                f.token_b.clone(),
                1_000i128,
                900i128,
                DEADLINE,
                path.clone(),
            )
                .into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert!(res.is_err());
    assert!(!matches!(res, Err(Ok(_))));

    // Positive control: the SAME args with the TRADER's authorization (plus
    // the nested token-transfer authorization) succeed — proving the failure
    // above was the missing signature, not malformed args or mocks.
    f.env.mock_auths(&[MockAuth {
        address: &f.trader,
        invoke: &MockAuthInvoke {
            contract: &f.router_id,
            fn_name: "swap",
            args: (
                f.trader.clone(),
                f.token_a.clone(),
                f.token_b.clone(),
                1_000i128,
                900i128,
                DEADLINE,
                path.clone(),
            )
                .into_val(&f.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &f.token_a,
                fn_name: "transfer",
                args: (f.trader.clone(), f.router_id.clone(), 1_000i128).into_val(&f.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let out = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, Ok(Ok(1_000)));
}

#[test]
fn negative_amount_in_fails() {
    let f = setup();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &-100, &1, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::ZeroAmount)));
}

#[test]
fn negative_hop_min_fails() {
    let f = setup();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, -5)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InvalidMinOut)));
}

#[test]
fn negative_global_min_fails() {
    let f = setup();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &-1, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InvalidMinOut)));
}

#[test]
fn unregistered_token_address_fails_safely() {
    let f = setup();
    let ghost = Address::generate(&f.env); // no contract lives here
    let path = sorovec![&f.env, step(&f, &ghost, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &ghost, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    // Host-level trap (no token contract), never a success, never a panic.
    assert!(res.is_err());
    assert!(!matches!(res, Err(Ok(_))));
}

#[test]
fn same_token_single_hop_round_trips() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_a, &f.adapter_id, 50_000);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_a, 900)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_a, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000);
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
}

#[test]
fn zero_hop_min_defers_to_global_min() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    // Hop floor 0, global floor 900, delivery 1000: success.
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 0)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000);
}

#[test]
fn large_values_round_trip_exactly() {
    let f = setup();
    // Whale scale: 10^18 units (1M whole tokens at 12 decimals).
    const WHALE: i128 = 1_000_000_000_000_000_000;
    mint(&f.env, &f.token_a, &f.trader, 2 * WHALE);
    mint(&f.env, &f.token_b, &f.adapter_id, 2 * WHALE);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, WHALE)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &WHALE, &WHALE, &DEADLINE, &path,
    );
    assert_eq!(out, WHALE);
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), WHALE);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), WHALE);
}

// -- Adapter admin gating ----------------------------------------------------------

#[test]
fn adapter_management_requires_adapter_admin() {
    let env = Env::default(); // no mocks at all
    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let adapter_id = env.register(TestAdapter, ());
    let client = stellariq_test_adapter::TestAdapterClient::new(&env, &adapter_id);
    client.initialize(&admin); // no auth needed
    let pool = Address::generate(&env);

    assert!(client.try_set_rate(&stranger, &pool, &1, &1).is_err());
    assert!(client.try_set_fail(&stranger, &pool, &true).is_err());
    assert!(client.try_set_misreport(&stranger, &pool, &1).is_err());
}

#[test]
fn router_upgrade_requires_admin() {
    let env = Env::default(); // no mocks at all
    let admin = Address::generate(&env);
    let router_id = env.register(super::Router, ());
    let client = super::RouterClient::new(&env, &router_id);
    client.initialize(&admin);
    let hash = soroban_sdk::BytesN::<32>::from_array(&env, &[9u8; 32]);
    // No admin signature available: must fail (and must not brick anything).
    assert!(client.try_upgrade(&hash).is_err());
    assert_eq!(client.get_admin(), admin);
}
