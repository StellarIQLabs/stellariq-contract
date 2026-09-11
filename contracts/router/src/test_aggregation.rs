#![cfg(test)]

//! Route-execution (aggregation) tests: the router receives an already
//! computed route and must execute it in order, deterministically, with full
//! atomicity. A separate aggregator contract is unnecessary BECAUSE this
//! behavior is proven here: ordered dispatch, chained amounts, per-hop and
//! global minimums, and all-or-nothing settlement (see `docs/architecture.md`
//! §2.3 for the decision record).

use super::fixture::*;
use soroban_sdk::vec as sorovec;
use stellariq_interfaces::RouterError;

// -- Ordered deterministic execution --------------------------------------------

#[test]
fn two_hop_swap_chains_amounts_across_pools() {
    let f = setup();
    // pool1: A->B at 3:2 ; pool2: B->C at 1:1.
    adapter(&f).set_rate(&f.admin, &f.pool, &3, &2);

    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);

    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 1_400),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 1_400),
    ];
    // 1000 A -> 1500 B -> 1500 C.
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &1_400, &DEADLINE, &path,
    );
    assert_eq!(out, 1_500);

    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 9_000);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 0); // intermediate never touches trader
    assert_eq!(balance(&f.env, &f.token_c, &f.trader), 1_500);
    assert_eq!(balance(&f.env, &f.token_b, &f.router_id), 0);
    assert_eq!(balance(&f.env, &f.token_c, &f.router_id), 0);
}

#[test]
fn three_hop_truncation_is_deterministic() {
    let f = setup();
    // 1000 -> /3 -> /3 with integer truncation at every hop.
    adapter(&f).set_rate(&f.admin, &f.pool, &1, &3);
    adapter(&f).set_rate(&f.admin, &f.pool2, &1, &3);

    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);

    // A->B (pool, 1000 -> 333), B->C (pool2, 333 -> 111), C->A?? No:
    // use A->B, B->A... simpler: A->B, B->C needs C inventory only.
    // Third hop: C->B on pool (1:3): 111 -> 37 B delivered to trader.
    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 300),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 100),
        step_on_pool(&f.pool, &f.token_c, &f.token_b, 30),
    ];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &30, &DEADLINE, &path,
    );
    // 1000*1/3 = 333 ; 333*1/3 = 111 ; 111*1/3 = 37. Truncated, never rounded.
    assert_eq!(out, 37);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 37);
}

#[test]
fn max_hops_boundary_succeeds() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    // The adapter must hold inventory it delivers each hop (net-zero per hop).
    mint(&f.env, &f.token_a, &f.adapter_id, 50_000);
    // Five 1:1 self-loop hops on alternating pools.
    let pools = [&f.pool, &f.pool2, &f.pool, &f.pool2, &f.pool];
    let mut path = sorovec![&f.env];
    for pool in pools {
        path.push_back(step_on_pool(pool, &f.token_a, &f.token_a, 0));
    }
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_a, &1_000, &1_000, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000);
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
}

// -- Failure handling: all-or-nothing --------------------------------------------

#[test]
fn failed_intermediate_hop_reverts_atomically() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);
    // Second venue rejects execution after the first hop already "succeeded".
    adapter(&f).set_fail(&f.admin, &f.pool2, &true);

    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 900),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 900),
    ];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::SwapFailed)));

    // Atomicity: trader keeps everything, no partial fill anywhere.
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 0);
    assert_eq!(balance(&f.env, &f.token_c, &f.trader), 0);
    assert_eq!(balance(&f.env, &f.token_a, &f.router_id), 0);
    assert_eq!(balance(&f.env, &f.token_b, &f.router_id), 0);
}

#[test]
fn mid_route_slippage_reverts_atomically() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);
    // First hop fine 1:1, second hop halves: 1000 -> 1000 -> 500 < 900 min.
    adapter(&f).set_rate(&f.admin, &f.pool2, &1, &2);

    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 900),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 1),
    ];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientOutput)));
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
    assert_eq!(balance(&f.env, &f.token_c, &f.trader), 0);
}

#[test]
fn hop_min_violation_mid_route_reverts() {
    let f = setup();
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);

    // Hop 2 delivers 1000 but demands 1001.
    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 900),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 1_001),
    ];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientOutput)));
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
}

#[test]
fn illiquid_intermediate_venue_fails_safely() {
    let f = setup();
    // Adapter holds B but NO C: second-hop delivery cannot settle.
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);

    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 900),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 900),
    ];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::SwapFailed)));
    assert_eq!(balance(&f.env, &f.token_a, &f.trader), 10_000);
    assert_eq!(balance(&f.env, &f.token_c, &f.trader), 0);
}

#[test]
fn execution_does_not_depend_on_offchain_state() {
    // Same route executed twice yields identical results: no oracle, no
    // API, no hidden input — only stored rates and on-chain balances.
    let f = setup();
    adapter(&f).set_rate(&f.admin, &f.pool, &3, &2);
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 40_000);

    let run = || {
        let path = sorovec![&f.env, step_on_pool(&f.pool, &f.token_a, &f.token_b, 1_400),];
        router(&f).swap(
            &f.trader, &f.token_a, &f.token_b, &1_000, &1_400, &DEADLINE, &path,
        )
    };
    assert_eq!(run(), 1_500);
    assert_eq!(run(), 1_500);
    assert_eq!(balance(&f.env, &f.token_b, &f.trader), 3_000);
}
