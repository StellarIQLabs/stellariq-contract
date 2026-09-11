# StellarIQ Contract Architecture (v0.1 — MVP)

Status: accepted for initial testnet release.
Scope: intentionally minimal. One deployed contract. No separate aggregator.

## 1. System context

```
stellariq-app            stellariq-data              stellariq-contracts
 (wallet + UI)           (off-chain routing engine)   (on-chain execution)
      │                         │                              │
      │  1. requests quote      │                              │
      ├────────────────────────►│                              │
      │  2. ranked route        │                              │
      │◄────────────────────────┤                              │
      │                         │                              │
      │  3. builds + signs swap tx (route embedded)            │
      ├──────────────────────────────────────────────────────►│
      │                         │                    Stellar / Soroban
      │  4. confirmation + events│                              │
      │◄──────────────────────────────────────────────────────┤
      │                         │                              │
      │                    5. indexes events                    │
      │                         │◄─────────────────────────────┤
```

Responsibility split (non-negotiable):

| Concern | Owner | Rationale |
|---|---|---|
| Route discovery, ranking, quotes | `stellariq-data` (off-chain) | Requires market data, simulations, floating point — unsuitable for deterministic contracts |
| Safe on-chain execution | `stellariq-contracts` (this repo) | Must be trust-minimized and atomic |
| Signing, tx construction | `stellariq-app` + user wallet | Private keys never touch contracts or backend |
| Settlement accounting, event log | Soroban contracts | Deterministic, auditable |

The contract layer MUST NOT depend on `stellariq-data` (or any centralized API)
being available at execution time. Once a transaction is constructed, execution
is fully self-contained on Stellar.

## 2. Contracts

### 2.1 Router (`contracts/router`) — the only deployed contract

Single entry point for all swaps. Responsibilities:

- Receive an already-computed route (`Vec<SwapStep>`) from the caller.
- Validate the route (continuity, bounds, registered protocols).
- Enforce trader authorization (`trader.require_auth()`).
- Enforce deadline (`deadline >= ledger.timestamp`), reject expired txs.
- Enforce per-hop minimums AND a global `amount_out_min` (slippage protection).
- Pull input tokens from the trader (SEP-41 `transfer`, no allowance games).
- Execute hops in order through registered protocol adapters.
- Emit `hop_executed` per hop and one `swap_executed` summary event.
- Fail atomically: any violation rolls back the whole swap (Soroban semantics).

Explicitly OUT of scope for the router:

- Price discovery / ranking (belongs to `stellariq-data`).
- Custody of user funds beyond transient per-transaction holding.
- Arbitrary external calls. The router calls exactly two external surfaces:
  1. SEP-41 token contracts (`transfer` only), and
  2. registered adapter contracts, fixed `swap(...)` entrypoint only.
- Protocol fees (deferred; see §7).
- Token allowlisting (deferred; see §7).

### 2.2 Protocol adapters (interface only in v0.1)

Each supported DEX/AMM is integrated behind a narrow adapter interface:

```rust
fn swap(env, pool, token_in, token_out, amount_in, amount_out_min, recipient) -> amount_out
```

- The router maps `protocol_id: Symbol -> adapter: Address` (admin-managed registry).
- A `SwapStep` names a `protocol`, never a raw executable target.
- Adapters encapsulate all DEX-specific call shapes; the router never learns them.
- v0.1 ships NO production adapters (Soroswap/Phoenix adapters are deferred work).
  A `contracts/test-adapter` exists SOLELY for the test suite and is never deployed.

### 2.3 Why no separate aggregator contract

Considered and rejected for MVP:

- The router already performs ordered multi-hop execution, which IS aggregation.
- A second contract would add a cross-contract trust hop, double auth surface,
  and deployment/upgrade coordination for zero functional gain.
- If future needs arise (e.g. split-across-venues execution, flash accounting),
  an aggregator can be introduced WITHOUT changing the router's external interface.

## 3. Trust assumptions

1. **Trader authorizes exactly what they sign.** Stellar auth binds `trader` to the
   precise `swap(...)` arguments; tampering invalidates the signature.
2. **Admin is trusted but constrained.** Admin can pause, manage the protocol
   registry, rotate admin, upgrade code. Admin CANNOT move user tokens, bypass
   `require_auth`, forge events, or change history.
3. **Adapters are untrusted for reporting, trusted only for delivery.**
   Every hop's delivery is verified on-chain via recipient balance deltas and
   only verified amounts are chained; inflated reports fail closed
   (`SwapFailed`) and any shortfall reverts the whole swap. A malicious
   adapter can therefore only deny service (grief), never steal: no token
   loss is possible, and registry removal contains the griefing. (Hardened
   during the security review; proven by misreport tests.)
4. **SEP-41 token contracts behave** (standard transfer semantics, no fee-on-transfer
   hooks that break accounting — see known limitations §7).
5. **Ledger timestamp** is the time oracle for deadlines (Stellar-close-time based,
   coarse but manipulation-resistant for swap-scale windows).

## 4. Admin capabilities (all auth-gated, all event-emitting)

| Capability | Function | Notes |
|---|---|---|
| Initialize once | `initialize(admin)` | One-shot; re-init rejected |
| Rotate admin | `set_admin(new)` | Old admin auth; zero-address rejected |
| Pause / resume | `set_paused(bool)` | Halts `swap` only; admin fns stay live |
| Register protocol | `set_protocol(id, adapter)` | Binds DEX integration point |
| Unregister protocol | `remove_protocol(id)` | Emergency containment |
| Upgrade code | `upgrade(hash)` | `update_current_contract_wasm`, admin auth |

## 5. Upgrade assumptions

- Code upgrade via Soroban native `update_current_contract_wasm`, admin-gated.
- Storage schema is versioned (`Config` carries no version field in v0.1; any
  breaking storage change REQUIRES a migration plan + testnet redeploy note).
- No auto-upgrades, no time-locks in v0.1 (documented limitation; acceptable for
  testnet MVP, required before mainnet).

## 6. Asset handling

- Assets are SEP-41 token contract addresses (`Address`).
- Amounts are raw `i128` smallest-unit integers. NO floating point anywhere.
- The router NEVER converts decimals: 7-decimal XLM and 12-decimal custom tokens
  are passed through opaquely; quote math (in `stellariq-data`) must use raw units.
- No custody: router balance is transient within one transaction (pull → forward
  per hop → deliver final output directly to trader).

## 7. Authorization model

- `swap`: `trader.require_auth()` — the trader signs the exact route + minimums.
- Admin fns: `admin.require_auth()` (current admin, read from storage).
- Token movement trader→router: SEP-41 `transfer` invoked by the router; auth
  propagates from the trader's top-level signature (standard Soroban auth).
- Router→adapter forwarding and adapter→recipient delivery: contracts acting on
  their own balances need no user signature (Soroban "current contract" auth).
- No allowances required from users (`approve` never needed — safer UX).

## 8. Error model

Contract errors (`RouterError`, `#[contracterror]`, stable discriminants):

| Code | Name | Meaning |
|---|---|---|
| 1 | AlreadyInitialized | `initialize` called twice |
| 2 | NotInitialized | any fn before `initialize` |
| 3 | NotAuthorized | caller is not admin |
| 4 | Paused | `swap` while paused |
| 5 | Expired | `deadline < ledger.timestamp` |
| 6 | ZeroAmount | `amount_in <= 0` or hop input `<= 0` |
| 7 | InvalidMinOut | `amount_out_min <= 0` |
| 8 | EmptyPath | route has no steps |
| 9 | TooManyHops | route exceeds `MAX_HOPS` (5) |
| 10 | DiscontinuousPath | hop tokens don't chain / endpoints mismatch |
| 11 | ProtocolNotFound | step references unregistered protocol |
| 12 | InsufficientOutput | hop or final output `<` minimum (slippage) |
| 13 | SwapFailed | adapter returned failure / non-positive output |
| 14 | InvalidAdmin | zero/None admin address |
| 15 | Overflow | checked-arithmetic violation |
| 16 | InsufficientBalance | trader balance below `amount_in` (deterministic pre-check) |

All financial failures are safe-fail (full rollback, no partial fills).

## 9. Event model (indexer contract with `stellariq-data`)

Topics are versioned (`v1:` prefix). Payloads are documented in `docs/events.md`.

- `swap_executed`: full settlement record (trader, in/out assets + amounts,
  protocol path, execution id, ledger context).
- `hop_executed`: per-hop record (links to execution id).
- Lifecycle: `initialized`, `admin_changed`, `paused`/`unpaused`,
  `protocol_set`, `protocol_removed`, `upgraded`.

## 10. External dependencies

| Dependency | Use | Pinned where |
|---|---|---|
| `soroban-sdk` | contract framework | workspace `Cargo.toml` |
| SEP-41 token contracts | asset movement (`transfer` only) | interface, not address |
| Adapter contracts | per-protocol execution | runtime registry, admin-set |
| `stellar-cli` / RPC | build + deploy tooling | `docs/` + `scripts/` |

No oracle, no bridge, no off-chain signer required at execution time.

## 11. Security assumptions (expanded in `docs/security-review.md`)

- Integer-only math with checked ops; `i128` bounds respected.
- Bounded loops (`MAX_HOPS = 5`); no unbounded iteration over user input.
- No `unwrap` on fallible paths; no `panic!` for expected errors.
- No delegatecall-style arbitrary invocation; fixed entrypoint + registry targets.
- Pause is a circuit breaker, not a fund control (there are no resident funds).
- Known limitations (accepted for testnet v0.1):
  - No per-token allowlist (permissionless execution).
  - No protocol fees.
  - No upgrade time-lock.
  - Fee-on-transfer / rebasing tokens unsupported (accounting assumes exact transfer).
  - Adapters for live DEXes not yet implemented (execution path proven via test adapter).
