# StellarIQ Contracts — Security Review (v0.1 testnet)

Date: 2026-09-11. Scope: `contracts/router`, `contracts/test-adapter`
(test-only), `interfaces`. Method: manual code review against the checklist in
the build task + targeted attack-case tests. All HIGH findings below are
resolved in this commit; open items are tracked as known limitations.

## 1. Threat model

**Assets at risk:** trader input tokens during execution (transient), correctness
of settlement accounting (events the indexer trusts), admin authority over the
protocol registry and code.

**Actors:**

| Actor | Power | Assumed behavior |
|---|---|---|
| Trader | signs exact `swap` args; holds input tokens | self-interested, benign |
| Admin | pause, registry, admin rotation, code upgrade | trusted but fallible; key compromise in scope |
| Adapter contract | receives hop input, delivers hop output | UNTRUSTED for reporting (may lie, fail, grief) |
| SEP-41 token contracts | debit/credit balances | standard semantics assumed (see limitations) |
| Network / ledger timestamp | close-time oracle for deadlines | honest-majority Stellar validators |
| `stellariq-data` / `stellariq-app` | quote routes, build txs | may be DOWN at execution (must not matter) |

**Out of scope for v0.1:** mainnet deployment, external audit, live-DEX adapter
implementations, privacy (all inputs are public by design).

## 2. Authentication & authorization — PASS

- `swap` calls `trader.require_auth()` FIRST thing after pause/init checks:
  Stellar binds the signature to the exact route, minimums and deadline.
  Tampering with any byte invalidates authorization. Proven by
  `unauthorized_admin_call_fails` (no-mock negative test) and blanket-mock
  positive tests.
- All management functions (`set_admin`, `set_paused`, `set_protocol`,
  `remove_protocol`, `upgrade`) require the STORED admin's auth. There is no
  backdoor role, no hard-coded admin, no hard-coded address (verified by grep).
- Token movement uses SEP-41 `transfer` (no `approve`/`allowance` UX, no
  allowance-griefing surface). Nested token auth propagates from the trader's
  top-level signature — the standard Soroban pattern wallets already support.
- `initialize` is permissionless by design (first-come). MITIGATION:
  deployment tooling MUST initialize atomically in the deploy flow
  (`scripts/deploy.sh` does deploy+initialize before any other use).
  Documented, not code-fixable without harming testability.

## 3. Attack surfaces & mitigations

| # | Surface | Analysis | Disposition |
|---|---|---|---|
| S-1 | Malicious adapter lies about output (esp. final hop) | HIGH. Pre-review code chained the adapter's RETURN value. An adapter delivering 0 while reporting 1M on the final hop would have settled "successfully" while the trader received nothing. Intermediate-hop lies self-destructed (next transfer underfunded → trap), but the final hop had no backstop. | FIXED: per-hop on-chain delivery verification via recipient balance delta; only verified amounts are checked, chained and emitted. Inflated reports → `SwapFailed`, revert. Proven by `inflated_report_fails_closed`, `inflated_report_mid_route_fails_atomically`, `deflated_report_settles_on_verified_delivery`. |
| S-2 | Unknown-protocol route wastes trader funds mid-execution | LOW (atomicity already prevented loss; only fee wasted). Protocol lookup happened inside the loop AFTER input was pulled. | FIXED: registry pre-check loop before any fund movement. |
| S-3 | Reentrant adapter calling back into the router | A reentered `swap` needs `trader.require_auth` for DIFFERENT args → host rejects (no auth entry). Reentered admin fns need admin auth → rejected. Reentered views are read-only. An adapter pulling router funds directly (`transfer` with `from=router`) fails: the router is not the direct invoker of that nested call, so invoker-based contract auth does not apply. Exposure is bounded to the already-forwarded hop input, which verification (S-1) covers. | Accepted by construction + host semantics. Not unit-testable under `mock_all_auths` (mocks allow everything); documented here instead. |
| S-4 | Integer overflow / precision loss | No floats anywhere (grep-verified). All rate math via `checked_mul_div` (returns `None`, never wraps). Nonce via `checked_add`. Release profile keeps `overflow-checks = true`. Division truncates toward zero — deterministic, documented, and pinned by `three_hop_truncation_is_deterministic`. | PASS |
| S-5 | Unbounded execution (gas griefing) | Route length capped at `MAX_HOPS = 5` (checked before validation). No loops over storage, no recursion. Registry writes are admin-only. | PASS |
| S-6 | Slippage / stale quotes | Global `amount_out_min > 0` MANDATORY (cannot opt out), per-hop minimums, `deadline` vs ledger timestamp (equality allowed, tested). Atomic revert on any breach — no partial fills, proven by balance assertions on every failure test. | PASS |
| S-7 | Panics / unwraps on adversarial input | `unwrap()`/`expect()`/`panic!` grep: ZERO occurrences in contract code (test code uses none either — all assertions are explicit `assert_eq!` on `try_` results). All fallible paths return `RouterError`. | PASS |
| S-8 | Arbitrary external calls | Router calls exactly: SEP-41 `transfer`/`balance` on trader-supplied TOKEN addresses (token interface is fixed and failures trap safely), and `swap` on REGISTRY-RESOLVED adapters via the generated typed client. No delegatecall-style primitive exists; `pool` is opaque data, never a call target. | PASS |
| S-9 | Front-running / sandwich | Contract-level: slippage minimums + deadlines are the defense; ordering protection belongs to the quote layer (`stellariq-data`) and Stellar ordering. Router adds no extractable edge (no resident liquidity to skew). | Accepted, documented |
| S-10 | Replay | No off-chain signatures consumed; Stellar sequence numbers prevent tx replay. No replayable claim exists in the contract. | PASS |
| S-11 | Storage bricking (TTL expiry) | Instance TTL extended on `initialize`, every admin write, and every `swap`; protocol entries get persistent TTL extension on registration. | PASS |
| S-12 | Fee-on-transfer / rebasing / hook tokens | Accounting assumes exact transfers. Such tokens fail SAFE (delivery verification / balance checks reject) but may fail spuriously. | Accepted limitation (documented; no code change — supporting them needs a different accounting model) |
| S-13 | Event forgery / indexer confusion | Events are emitted by the contract itself post-checks; reverted txs emit nothing (host semantics). Topics + `execution_id` join hops to swaps; schema versioned (`EVENT_SCHEMA_VERSION = 1`). | PASS |
| S-14 | Dust stranding via over-delivery | Chaining the VERIFIED delta (not the report) means over-delivery flows to the recipient, never strands. Router holds no balance past the tx in any tested path. | PASS |

## 4. Known limitations (accepted for testnet v0.1)

1. No per-token allowlist — execution is permissionless by design.
2. No protocol fees.
3. No upgrade time-lock — admin can replace code immediately. REQUIRED before
   mainnet (timelock + announcement discipline).
4. No live-DEX adapters shipped — execution proven via the test adapter; each
   production adapter needs its own review before registration.
5. Permissionless one-shot `initialize` — deploy flow must init atomically.
6. Ledger-timestamp deadlines are coarse (~5s); sub-ledger precision is not
   available on Stellar by design.
7. Fee-on-transfer / rebasing tokens unsupported (fail safe, not supported).

## 5. Remaining risks

- **Admin key compromise** → attacker can register a griefing adapter (denial
  of swaps, NOT theft — S-1 fix holds), pause the router, or push malicious
  code. Mitigation path: hardware-backed admin, then multisig + timelock
  before mainnet.
- **Production adapters** are new trusted code when written — each must be
  reviewed against this same checklist, ESPECIALLY honest-delivery behavior
  (verification catches lies, but a venue that silently keeps input AND
  delivers will pass verification while the venue itself stole — that is
  venue risk, outside the router's visibility).
- **No external audit yet** — required before mainnet. This document is a
  developer security pass, not a substitute.

## 6. Verification for this review

- `cargo fmt --all` — clean
- `cargo clippy --workspace --all-targets` — zero warnings
- `cargo test --workspace` — all pass (8 interfaces + 36 router, incl. 3 new
  delivery-verification tests that FAIL on the pre-fix code by construction:
  pre-fix, the inflated report chained 5000 and returned success)
- `stellar contract build` — clean, exports audited (`initialize`, `set_admin`,
  `set_paused`, `set_protocol`, `remove_protocol`, `upgrade`, views, `swap`)
- Dangerous-pattern grep (`unwrap|expect|panic!|unchecked|hard-coded
  secrets/keys/addresses`) — clean (one comment-only match)
