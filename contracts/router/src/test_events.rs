#![cfg(test)]

//! Event coverage for the `stellariq-data` indexer contract: every emitted
//! event is asserted EXACTLY (topics + data), hop records join to their swap
//! summary via a shared `execution_id`, and failed swaps emit nothing.

use super::fixture::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    vec as sorovec, Address, Env, IntoVal, Map, Symbol, Val, Vec,
};
use stellariq_interfaces::{RouterError, EVENT_SCHEMA_VERSION};

fn sym(env: &Env, s: &str) -> Val {
    Symbol::new(env, s).into_val(env)
}

fn router_events(f: &Fixture) -> soroban_sdk::testutils::ContractEvents {
    f.env.events().all().filter_by_contract(&f.router_id)
}

fn expected_swap(
    f: &Fixture,
    execution_id: u64,
    amount_in: i128,
    amount_out: i128,
    protocols: Vec<Symbol>,
) -> (Address, Vec<Val>, Val) {
    let topics: Vec<Val> = sorovec![
        &f.env,
        sym(&f.env, "swap_executed"),
        f.trader.clone().into_val(&f.env),
    ];
    let mut data = Map::<Symbol, Val>::new(&f.env);
    data.set(Symbol::new(&f.env, "amount_in"), amount_in.into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "amount_out"),
        amount_out.into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "execution_id"),
        execution_id.into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "ledger"),
        f.env.ledger().sequence().into_val(&f.env),
    );
    data.set(Symbol::new(&f.env, "protocols"), protocols.into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "token_in"),
        f.token_a.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "token_out"),
        f.token_b.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    (f.router_id.clone(), topics, data.into_val(&f.env))
}

#[allow(clippy::too_many_arguments)]
fn expected_hop(
    f: &Fixture,
    execution_id: u64,
    hop_index: u32,
    pool: &Address,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
    amount_out: i128,
) -> (Address, Vec<Val>, Val) {
    let topics: Vec<Val> = sorovec![
        &f.env,
        sym(&f.env, "hop_executed"),
        execution_id.into_val(&f.env),
    ];
    let mut data = Map::<Symbol, Val>::new(&f.env);
    data.set(Symbol::new(&f.env, "amount_in"), amount_in.into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "amount_out"),
        amount_out.into_val(&f.env),
    );
    data.set(Symbol::new(&f.env, "hop_index"), hop_index.into_val(&f.env));
    data.set(Symbol::new(&f.env, "pool"), pool.clone().into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "protocol"),
        symbol_short!("test").into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "token_in"),
        token_in.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "token_out"),
        token_out.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "trader"),
        f.trader.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    (f.router_id.clone(), topics, data.into_val(&f.env))
}

#[test]
fn swap_emits_joinable_hop_and_summary() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(out, 1_000);

    // First swap on this instance: execution_id 1, shared by hop + summary.
    let hop = expected_hop(&f, 1, 0, &f.pool, &f.token_a, &f.token_b, 1_000, 1_000);
    let summary = expected_swap(&f, 1, 1_000, 1_000, sorovec![&f.env, symbol_short!("test")]);
    assert_eq!(router_events(&f), sorovec![&f.env, hop, summary]);
}

#[test]
fn two_hop_swap_emits_ordered_hops_then_summary() {
    let f = setup();
    adapter(&f).set_rate(&f.admin, &f.pool, &3, &2);
    mint(&f.env, &f.token_a, &f.trader, 10_000);
    mint(&f.env, &f.token_b, &f.adapter_id, 20_000);
    mint(&f.env, &f.token_c, &f.adapter_id, 20_000);

    let path = sorovec![
        &f.env,
        step_on_pool(&f.pool, &f.token_a, &f.token_b, 1_400),
        step_on_pool(&f.pool2, &f.token_b, &f.token_c, 1_400),
    ];
    // 1000 A -> 1500 B -> 1500 C, global min 1400.
    let out = router(&f).swap(
        &f.trader, &f.token_a, &f.token_c, &1_000, &1_400, &DEADLINE, &path,
    );
    assert_eq!(out, 1_500);

    // NOTE: expected_swap hardcodes token_a/token_b endpoints; build the
    // two-hop summary inline here.
    let hop0 = expected_hop(&f, 1, 0, &f.pool, &f.token_a, &f.token_b, 1_000, 1_500);
    let hop1 = expected_hop(&f, 1, 1, &f.pool2, &f.token_b, &f.token_c, 1_500, 1_500);
    let topics: Vec<Val> = sorovec![
        &f.env,
        sym(&f.env, "swap_executed"),
        f.trader.clone().into_val(&f.env),
    ];
    let mut data = Map::<Symbol, Val>::new(&f.env);
    data.set(Symbol::new(&f.env, "amount_in"), 1_000i128.into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "amount_out"),
        1_500i128.into_val(&f.env),
    );
    data.set(Symbol::new(&f.env, "execution_id"), 1u64.into_val(&f.env));
    data.set(
        Symbol::new(&f.env, "ledger"),
        f.env.ledger().sequence().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "protocols"),
        sorovec![&f.env, symbol_short!("test"), symbol_short!("test")].into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "token_in"),
        f.token_a.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "token_out"),
        f.token_c.clone().into_val(&f.env),
    );
    data.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    let summary = (f.router_id.clone(), topics, data.into_val(&f.env));

    assert_eq!(router_events(&f), sorovec![&f.env, hop0, hop1, summary]);
}

#[test]
fn execution_ids_increase_across_swaps() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 900)];
    for expected_id in [1u64, 2u64] {
        router(&f).swap(
            &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
        );
        let hop = expected_hop(
            &f,
            expected_id,
            0,
            &f.pool,
            &f.token_a,
            &f.token_b,
            1_000,
            1_000,
        );
        let summary = expected_swap(
            &f,
            expected_id,
            1_000,
            1_000,
            sorovec![&f.env, symbol_short!("test")],
        );
        assert_eq!(router_events(&f), sorovec![&f.env, hop, summary]);
    }
}

#[test]
fn failed_swap_emits_nothing() {
    let f = setup();
    fund_single_hop(&f, 10_000, 10_000);
    adapter(&f).set_rate(&f.admin, &f.pool, &1, &2); // 1000 -> 500 < 900
    let path = sorovec![&f.env, step(&f, &f.token_a, &f.token_b, 1)];
    let res = router(&f).try_swap(
        &f.trader, &f.token_a, &f.token_b, &1_000, &900, &DEADLINE, &path,
    );
    assert_eq!(res, Err(Ok(RouterError::InsufficientOutput)));
    assert_eq!(router_events(&f), sorovec![&f.env]);
}

#[test]
fn lifecycle_events_carry_admin_and_config() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_id = env.register(super::Router, ());
    let client = super::RouterClient::new(&env, &router_id);

    client.initialize(&admin);
    // `admin` is a topic; data carries only the schema version.
    let mut data = Map::<Symbol, Val>::new(&env);
    data.set(
        Symbol::new(&env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&env),
    );
    assert_eq!(
        env.events().all().filter_by_contract(&router_id),
        sorovec![
            &env,
            (
                router_id.clone(),
                sorovec![&env, sym(&env, "initialized"), admin.clone().into_val(&env),],
                data.into_val(&env),
            ),
        ]
    );

    client.set_protocol(&symbol_short!("test"), &admin);
    // `protocol` is a topic; data carries adapter + version.
    let mut pdata = Map::<Symbol, Val>::new(&env);
    pdata.set(Symbol::new(&env, "adapter"), admin.clone().into_val(&env));
    pdata.set(
        Symbol::new(&env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&env),
    );
    assert_eq!(
        env.events().all().filter_by_contract(&router_id),
        sorovec![
            &env,
            (
                router_id.clone(),
                sorovec![
                    &env,
                    sym(&env, "protocol_set"),
                    symbol_short!("test").into_val(&env),
                ],
                pdata.into_val(&env),
            ),
        ]
    );
}

#[test]
fn management_events_cover_admin_pause_and_removal() {
    let f = setup();
    let next = Address::generate(&f.env);

    router(&f).set_admin(&next);
    let mut adata = Map::<Symbol, Val>::new(&f.env);
    adata.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    assert_eq!(
        router_events(&f),
        sorovec![
            &f.env,
            (
                f.router_id.clone(),
                sorovec![
                    &f.env,
                    sym(&f.env, "admin_changed"),
                    f.admin.clone().into_val(&f.env),
                    next.clone().into_val(&f.env),
                ],
                adata.into_val(&f.env),
            ),
        ]
    );

    router(&f).set_paused(&true);
    let mut paused_data = Map::<Symbol, Val>::new(&f.env);
    paused_data.set(Symbol::new(&f.env, "paused"), true.into_val(&f.env));
    paused_data.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    assert_eq!(
        router_events(&f),
        sorovec![
            &f.env,
            (
                f.router_id.clone(),
                sorovec![
                    &f.env,
                    sym(&f.env, "pause_changed"),
                    next.clone().into_val(&f.env),
                ],
                paused_data.into_val(&f.env),
            ),
        ]
    );

    router(&f).remove_protocol(&symbol_short!("test"));
    let mut rdata = Map::<Symbol, Val>::new(&f.env);
    rdata.set(
        Symbol::new(&f.env, "version"),
        EVENT_SCHEMA_VERSION.into_val(&f.env),
    );
    assert_eq!(
        router_events(&f),
        sorovec![
            &f.env,
            (
                f.router_id.clone(),
                sorovec![
                    &f.env,
                    sym(&f.env, "protocol_removed"),
                    symbol_short!("test").into_val(&f.env),
                ],
                rdata.into_val(&f.env),
            ),
        ]
    );
}
