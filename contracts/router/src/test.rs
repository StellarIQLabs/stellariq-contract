#![cfg(test)]

//! Core router tests: lifecycle, authorization, registry, single-hop
//! execution and every validation failure. Multi-hop aggregation scenarios
//! live in `test_aggregation.rs`; security/event coverage lands in Tasks 7–9.

use super::fixture::*;
use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    vec as sorovec, Address, Env,
};
use stellariq_interfaces::{AdapterError, MAX_HOPS};

// -- Lifecycle ---------------------------------------------------------------

#[test]
fn initialize_sets_admin_and_defaults() {
    let f = setup();
    assert_eq!(router(&f).get_admin(), f.admin);
    assert!(!router(&f).is_paused());
    assert_eq!(
        router(&f).version(),
        soroban_sdk::String::from_str(&f.env, env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn initialize_twice_fails() {
    let f = setup();
    let res = router(&f).try_initialize(&f.admin);
    assert_eq!(res, Err(Ok(RouterError::AlreadyInitialized)));
}

#[test]
fn use_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let router_id = env.register(Router, ());
    let client = RouterClient::new(&env, &router_id);
    assert_eq!(client.try_get_admin(), Err(Ok(RouterError::NotInitialized)));
    assert_eq!(client.try_is_paused(), Err(Ok(RouterError::NotInitialized)));
    assert_eq!(
        client.try_set_paused(&true),
        Err(Ok(RouterError::NotInitialized))
    );
}

// -- Admin -------------------------------------------------------------------

#[test]
fn set_admin_rotates_and_new_admin_governs() {
    let f = setup();
    let next = Address::generate(&f.env);
    router(&f).set_admin(&next);
    assert_eq!(router(&f).get_admin(), next);
    // With blanket auth mocks the new admin's management calls go through.
    router(&f).set_paused(&true);
    assert!(router(&f).is_paused());
}

#[test]
fn set_admin_to_self_is_invalid() {
    let f = setup();
    let res = router(&f).try_set_admin(&f.admin);
    assert_eq!(res, Err(Ok(RouterError::InvalidAdmin)));
}

#[test]
fn unauthorized_admin_call_fails() {
    // No auth mocks at all: require_auth for the stored admin cannot pass.
    let env = Env::default();
    let admin = Address::generate(&env);
    let router_id = env.register(Router, ());
    let client = RouterClient::new(&env, &router_id);
    client.initialize(&admin);
    // initialize needs no auth, so it succeeded; admin-gated calls must fail.
    assert_eq!(client.get_admin(), admin);
    assert!(client.try_set_paused(&true).is_err());
    assert!(client.try_set_admin(&Address::generate(&env)).is_err());
}

// -- Pause -------------------------------------------------------------------

#[test]
fn paused_router_rejects_swaps_but_admin_fns_stay_live() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    router(&f).set_paused(&true);
    assert!(router(&f).is_paused());

    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &5_000, &4_000, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::Paused)));

    // Admin management still works while paused.
    router(&f).set_paused(&false);
    assert!(!router(&f).is_paused());
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &5_000, &4_000, &DEADLINE, &path,
    );
    assert_eq!(out, 5_000);
}

// -- Registry -----------------------------------------------------------------

#[test]
fn unknown_protocol_fails() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    let bad = SwapStep {
        protocol: symbol_short!("nope"),
        pool: f.pool.clone(),
        token_in: f.token_a.clone(),
        token_out: f.token_b.clone(),
        amount_out_min: 1,
    };
    let path = sorovec![&f.env, bad];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::ProtocolNotFound)));
}

#[test]
fn removed_protocol_fails() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    router(&f).remove_protocol(&symbol_short!("test"));
    let res = router(&f).try_get_protocol(&symbol_short!("test"));
    assert_eq!(res, Err(Ok(RouterError::ProtocolNotFound)));

    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::ProtocolNotFound)));
}

#[test]
fn remove_missing_protocol_fails() {
    let f = setup();
    let res = router(&f).try_remove_protocol(&symbol_short!("ghost"));
    assert_eq!(res, Err(Ok(RouterError::ProtocolNotFound)));
}

// -- Single-hop success --------------------------------------------------------

#[test]
fn single_hop_swap_succeeds_and_settles() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);

    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000); // default 1:1 test-adapter rate

    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 9_000);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 1_000);
    // Router retains nothing; adapter forwarded input and paid output.
    assert_eq!(balance(&f.env, &f.token_a, &f.router_id), 0);
    assert_eq!(balance(&f.env, &f.token_b, &f.router_id), 0);
    assert_eq!(balance(&f.env, &f.token_a, &f.adapter_id), 1_000);
    assert_eq!(balance(&f.env, &f.token_b, &f.adapter_id), 9_000);
}

#[test]
fn deadline_equal_to_now_is_valid() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    f.env.ledger().set_timestamp(DEADLINE);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000);
}

// -- Validation failures -------------------------------------------------------

#[test]
fn zero_amount_fails() {
    let f = setup();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(&f.trader, &f.token_a, &f.token_b, &0, &1, &DEADLINE, &path);
    assert_eq!(res, Err(Ok(RouterError::ZeroAmount)));
}

#[test]
fn non_positive_min_out_fails() {
    let f = setup();
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &0, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InvalidMinOut)));
}

#[test]
fn expired_deadline_fails() {
    let f = setup();
    f.env.ledger().set_timestamp(DEADLINE + 1);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::Expired)));
}

#[test]
fn empty_path_fails() {
    let f = setup();
    let path = sorovec![&f.env];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::EmptyPath)));
}

#[test]
fn too_many_hops_fails() {
    let f = setup();
    let mut path = sorovec![&f.env];
    // MAX_HOPS + 1 self-looping steps (shape valid, length not).
    for _ in 0..MAX_HOPS + 1 {
        path.push_back(step(&f, &f.token_a, &f.token_a, 0));
    }
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_a, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::TooManyHops)));
}

#[test]
fn discontinuous_path_fails() {
    let f = setup();
    // A->B then C->B: middle break.
    let path = sorovec![
        &f.env,
        step(&f, &f.token_a, &f.token_b, 1),
        step(&f, &f.token_c, &f.token_b, 1),
    ];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::DiscontinuousPath)));
}

#[test]
fn endpoint_mismatch_fails() {
    let f = setup();
    // Route ends at C but swap declares B.
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_c, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::DiscontinuousPath)));
}

#[test]
fn insufficient_balance_fails() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 500); // less than amount_in
    mint(&f.env, &f.token_b, &f.adapter_id, 10_000);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientBalance)));
}

// -- Slippage / execution failures ----------------------------------------------

#[test]
fn global_min_out_violation_fails() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    // 1:2 rate -> 1000 in gives 500 out, below the 900 global minimum.
    adapter(&f).set_rate(&f.admin, &f.pool, &1, &2);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientOutput)));
}

#[test]
fn per_hop_min_violation_fails() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    // Hop min 1_001 can never be met by a 1_000 1:1 fill.
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1_001)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientOutput)));
}

#[test]
fn adapter_fault_maps_to_swap_failed() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    adapter(&f).set_fail(&f.admin, &f.pool, &true);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::SwapFailed)));
}

#[test]
fn adapter_zero_rate_maps_to_swap_failed() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    adapter(&f).set_rate(&f.admin, &f.pool, &0, &1);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::SwapFailed)));
}

// -- Adapter unit behavior (cross-crate sanity) ----------------------------------

#[test]
fn adapter_rejects_bad_rates_and_double_init() {
    let f = setup();
    let res = adapter(&f).try_set_rate(&f.admin, &f.pool, &1, &0);
    assert_eq!(res, Err(Ok(AdapterError::InvalidRate)));
    let res = adapter(&f).try_initialize(&f.admin);
    assert_eq!(res, Err(Ok(AdapterError::AlreadyInitialized)));
}
