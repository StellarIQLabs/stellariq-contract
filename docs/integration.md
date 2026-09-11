# StellarIQ App Integration Guide (`stellariq-app` → contracts)

Audience: a developer working ONLY in `stellariq-app`. No contract repo
checkout needed — everything to integrate is below. Deep dives:
`docs/architecture.md`, `docs/events.md`, `docs/testnet-deployment.md`.

## 1. Deployed contracts (testnet)

| Contract | ID | Notes |
|---|---|---|
| Router v0.1.0 | `CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP` | sole integration point |
| Test adapter | — | NEVER deployed (unit-test only) |

Network: testnet (`Test SDF Network ; September 2015`,
RPC `https://soroban-testnet.stellar.org`). Mainnet: not deployed; do not
hard-code a mainnet id — read it from `deployments/` when it exists.

## 2. Intended flow

```
stellariq-app
      │
      ▼
Get quote from stellariq-data          (off-chain: discovery + ranking)
      │
      ▼
Receive route                          (ordered hops + per-hop minimums)
      │
      ▼
Build contract transaction             (swap(...) args, this doc §4)
      │
      ▼
User wallet signs                      (Freighter etc.; trader = user)
      │
      ▼
Submit to Stellar                      (Simulate → Sign → Send)
      │
      ▼
Monitor confirmation                   (success: swap_executed event)
      │
      ▼
stellariq-data indexes event           (see docs/events.md)
```

The app NEVER needs `stellariq-data` to be up at submit time: the route is
fully embedded in the transaction.

## 3. Contract methods (router)

| Method | Args | Returns | Auth |
|---|---|---|---|
| `swap` | `trader: Address, token_in: Address, token_out: Address, amount_in: i128, amount_out_min: i128, deadline: u64, path: Vec<SwapStep>` | `i128` final output | `trader` signs |
| `version` | — | `String` (`"0.1.0"`) | none |
| `get_admin` | — | `Address` | none |
| `is_paused` | — | `bool` | none |
| `get_protocol` | `protocol: Symbol` | `Address` (adapter) | none |

Admin-only (`set_admin`, `set_paused`, `set_protocol`, `remove_protocol`,
`upgrade`): app-facing wallets must NEVER call these; they exist for ops.

`SwapStep` (one route leg):
`{ protocol: Symbol, pool: Address, token_in: Address, token_out: Address, amount_out_min: i128 }`.

## 4. Building the `swap` transaction

1. **Quote:** ask `stellariq-data` for a route. It returns ordered legs with a
   per-leg `amount_out_min` and a global `amount_out_min`.
2. **Validate client-side** (cheap pre-flight, contract re-checks everything):
   - `amount_in > 0`, `amount_out_min > 0` (mandatory — cannot opt out);
   - `path` non-empty, ≤ 5 hops; legs chain
     (`legs[i].token_out == legs[i+1].token_in`), first `token_in` and last
     `token_out` match the swap assets;
   - `deadline` = now + small window (ledger close-time granularity ~5s;
     expired txs revert with `Expired`). Do NOT use huge deadlines.
   - all amounts are RAW smallest-unit integers (`i128`). Convert from display
     units using each token's decimals (query the token contract). No floats.
   - every `protocol` in the route must currently resolve: call
     `get_protocol(protocol)` — it errors with `ProtocolNotFound` (11) if the
     venue was unregistered. Refresh the quote in that case.
3. **Check `is_paused()`**: `true` means swaps revert — show maintenance UI.
4. **Assemble + simulate** via RPC (`simulateTransaction`): simulation builds
   the auth entries (the trader authorizes the nested token transfer) and the
   footprint. Surface the simulated `amount_out` vs `amount_out_min` and the
   fee to the user BEFORE asking for a signature.
5. **Sign + send.** `trader` MUST be the user's wallet address.
6. **Confirm:** success returns the final output amount AND emits
   `swap_executed` (+ per-hop `hop_executed`). Parse events per
   `docs/events.md` to render the receipt. Any revert returns funds untouched
   (atomic — no partial fills, ever).

No `approve`/allowance step is needed: the router pulls via `transfer` under
the swap's own authorization. Do not invent one.

## 5. Error codes (`RouterError`, stable — never renumbered)

| Code | Meaning | App action |
|---|---|---|
| 1/2 | AlreadyInitialized / NotInitialized | ops issue — alert, don't retry |
| 3 | NotAuthorized | wrong signer / admin-only fn — fix caller |
| 4 | Paused | maintenance UI |
| 5 | Expired | rebuild with fresh deadline |
| 6/7 | ZeroAmount / InvalidMinOut | fix amounts (min must be > 0) |
| 8/9 | EmptyPath / TooManyHops | re-quote (≤ 5 hops) |
| 10 | DiscontinuousPath | route corrupted — re-quote, never patch legs by hand |
| 11 | ProtocolNotFound | venue unregistered — re-quote |
| 12 | InsufficientOutput | slippage — re-quote / widen tolerance in `stellariq-data` |
| 13 | SwapFailed | venue failed — re-quote via another venue |
| 14/15 | InvalidAdmin / Overflow | ops / retry with sane amounts |
| 16 | InsufficientBalance | fund the wallet first |

Anything else (host errors, simulation failure): treat as transient or fatal
per RPC guidance, funds are safe.

## 6. Wallet signing flow (Freighter-style)

- `sourceAccount` = user wallet; `trader` arg = same address.
- ALWAYS `simulateTransaction` first: it produces the Soroban auth entries the
  wallet must sign (trader authorizing the router call AND the nested
  input-token transfer). Skipping simulation is the #1 integration bug.
- Show the user: in/out assets + amounts (raw → display conversion),
  `amount_out_min` (worst acceptable), deadline (human time), network
  (testnet label until mainnet exists).
- After send, poll tx status; on `SUCCESS` read back `swap_executed` for the
  receipt (input/output deltas + `execution_id` for support).

## 7. Network configuration

```ts
const STELLARIQ = {
  network: "testnet",
  passphrase: "Test SDF Network ; September 2015",
  rpc: "https://soroban-testnet.stellar.org",
  router: "CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP",
};
```

## 8. Worked example (stellar-cli, read-only + failure shapes)

```sh
CID=CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP
stellar contract invoke --id $CID --source-account <YOU> --network testnet -- version
stellar contract invoke --id $CID --source-account <YOU> --network testnet -- is_paused
stellar contract invoke --id $CID --source-account <YOU> --network testnet \
  -- get_protocol --protocol test
# ^ errors ProtocolNotFound (11) while no venue is registered — expected.
```

A JS (stellar-sdk) `swap` assembly follows the same arg order as §3; encode
`path` as a Vec of maps with the five `SwapStep` fields. Keep the SDK pinned
to a version supporting protocol 27+ contract invocation.
