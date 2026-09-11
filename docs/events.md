# StellarIQ Contract Events — Indexer Contract (`stellariq-data`)

All event schemas are defined in code in `interfaces/src/lib.rs` (the single
source of truth) and pinned by exact-match tests in
`contracts/router/src/test_events.rs`. This document is the human/operator
reading guide for the indexer team. If this doc and the code ever disagree,
THE CODE WINS — then fix this doc.

- Schema version: `EVENT_SCHEMA_VERSION = 1` (every event carries `version: u32`).
- Event names are the `snake_case` struct names, emitted as the FIRST topic.
- Data payloads are Soroban maps keyed by field name (topic fields excluded).
- Amounts are raw `i128` smallest-unit integers. No floats, no decimals metadata
  (resolve decimals per token contract off-chain).
- Failed transactions emit NOTHING (host rolls back events with state).
- No sensitive data in events: addresses and amounts only — no keys, no memos,
  no off-chain identifiers.

## `swap_executed` — the primary settlement record

One per successful `swap`. Everything needed to reconstruct swap activity:

| Position | Field | Type | Notes |
|---|---|---|---|
| topic 0 | `swap_executed` | Symbol | event discriminator |
| topic 1 | `trader` | Address | filter: all swaps by trader |
| data | `token_in` / `token_out` | Address | SEP-41 contracts |
| data | `amount_in` / `amount_out` | i128 | VERIFIED delivered amounts |
| data | `protocols` | Vec<Symbol> | ordered venue path, e.g. `["soroswap","phoenix"]` |
| data | `execution_id` | u64 | per-contract monotonic id (starts at 1) |
| data | `ledger` | u32 | ledger sequence (timestamp via ledger header) |
| data | `version` | u32 | schema version (1) |

## `hop_executed` — per-hop leg records

One per hop, in order (`hop_index` 0..n), emitted BEFORE the summary:

| Position | Field | Type | Notes |
|---|---|---|---|
| topic 0 | `hop_executed` | Symbol | event discriminator |
| topic 1 | `execution_id` | u64 | JOIN KEY to `swap_executed` |
| data | `trader` | Address | redundant on purpose (partition-friendly) |
| data | `hop_index` | u32 | position in route |
| data | `protocol` / `pool` | Symbol / Address | venue + market |
| data | `token_in` / `token_out` | Address | leg assets |
| data | `amount_in` / `amount_out` | i128 | verified leg amounts |
| data | `version` | u32 | schema version (1) |

Join rule: `hop_executed[topic execution_id] == swap_executed[data execution_id]`
AND same contract id AND same ledger. Both are emitted by the same contract in
the same transaction, so grouping by `(contract_id, ledger, execution_id)` is
exact. `execution_id` is allocated once per swap and shared by all its hops —
regression-tested (`swap_emits_joinable_hop_and_summary`).

## Lifecycle events (governance audit trail)

| Event | Topics | Data | Emitted by |
|---|---|---|---|
| `initialized` | `[initialized, admin]` | `{version}` | `initialize` (once) |
| `admin_changed` | `[admin_changed, old_admin, new_admin]` | `{version}` | `set_admin` |
| `paused` / `unpaused` | see below | `{admin, paused, version}` | `set_paused` |

NOTE: the pause event is named `pause_changed` (struct `PauseChanged`), with
`paused: bool` in data telling the direction — do NOT filter on two names.

| Event | Topics | Data | Emitted by |
|---|---|---|---|
| `pause_changed` | `[pause_changed, admin]` | `{paused, version}` | `set_paused` |
| `protocol_set` | `[protocol_set, protocol]` | `{adapter, version}` | `set_protocol` |
| `protocol_removed` | `[protocol_removed, protocol]` | `{version}` | `remove_protocol` |
| `upgraded` | `[upgraded, admin]` | `{wasm_hash, version}` | `upgrade` |

## Indexer ingestion checklist (`stellariq-data`)

1. Subscribe to the router contract id(s) in `deployments/*.json`.
2. Ingest `swap_executed` as the canonical swap row; attach hops by
   `(contract_id, ledger, execution_id)`.
3. Sanity-check each swap: `len(hops) == len(protocols)`, hop chaining
   (`hops[i].token_out == hops[i+1].token_in`), first/last endpoints match the
   summary, `hop_index` dense from 0. Mismatches indicate a schema change —
   alert, don't silently coerce.
4. Track lifecycle events for the ops dashboard (admin changes, pauses,
   protocol registry changes, upgrades invalidate cached specs).
5. Branch parsing on `data.version`, NOT on code deploys: v1 shapes are frozen
   above; any new shape bumps `EVENT_SCHEMA_VERSION` and will be documented
   here before deployment.
6. Ledger timestamp comes from the ledger header, never from events.

## Worked example (single hop, 1:1 test venue)

```
topics: [swap_executed, GTRADER...]
data:   { token_in: CTOKEN_A, token_out: CTOKEN_B,
          amount_in: 1000, amount_out: 1000,
          protocols: ["test"], execution_id: 1, ledger: 0, version: 1 }

topics: [hop_executed, 1]
data:   { trader: GTRADER, hop_index: 0, protocol: "test", pool: CPOOL...,
          token_in: CTOKEN_A, token_out: CTOKEN_B,
          amount_in: 1000, amount_out: 1000, version: 1 }
```

## Version history

- v1 (2026-09-11, router 0.1.0): initial schema. `execution_id` allocated once
  per swap and shared by hops + summary (fixed pre-release: an early build
  used the pre-increment nonce for hops and post-increment for the summary —
  caught by exact-match event tests before any deployment).
