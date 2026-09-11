# Testnet Deployment & Verification (v0.1.0)

## Deployment

| Field | Value |
|---|---|
| Network | Stellar testnet (`Test SDF Network ; September 2015`) |
| Contract | `router` (the ONLY deployed contract; test adapter never deployed) |
| Contract ID | `CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP` |
| Deployed at (UTC) | 2026-09-11T12:24:22Z |
| Deploy tx | `3b4d13d4169e95859d810c50204ef509210c6eb80f98a5955b40c6bde2695b00` |
| Deploy ledger | 4621174 (status SUCCESS) |
| Init tx | `8afc0b9389d978e1e17efcbd06ac9e06d5cb1e572417f08ea063bdaeb3226dfa` |
| Git commit | `1d399eb` (`feat: add StellarIQ deployment tooling`) |
| Contract version | `0.1.0` |
| WASM sha256 | `1a3d03c5775b1ac532559aa89a5e1a5d84c5b46a72d40e13b1b3071c97d1e750` |
| Admin | `GCASCW5GJ3364OPCO4R57AXE3OHSBQOIHLFL5RFVWJIIAUQPAZGK42R4` (deployer identity) |
| Deployer identity | `stellariq-deployer` (local `stellar keys`, friendbot-funded; key material never leaves `~/.config/stellar`) |
| Configuration | unpaused, empty protocol registry (adapters registered per integration) |

NOTE: verify on [stellar.expert/testnet](https://stellar.expert/explorer/testnet)
via the deploy-tx hash or contract ID — the contract ID + init event are the
primary references.

Canonical machine-readable record: `deployments/testnet-router.json`.

## Verification results (all live, 2026-09-11)

| Check | Method | Result |
|---|---|---|
| Contract exists | deploy tx SUCCESS @ ledger 4621174 | PASS |
| Initialization correct | init tx emitted `Initialized(admin, version 1)` | PASS |
| Admin correct | `get_admin()` == metadata admin | PASS |
| Version correct | `version()` == `0.1.0` | PASS |
| Not paused | `is_paused()` == `false` | PASS |
| Pause circuit-breaker | `set_paused(true)` → `true` → `set_paused(false)` → `false`, with `PauseChanged` events | PASS |
| Unknown protocol rejected | `get_protocol(no_such_protocol)` errors | PASS |
| Invalid route fails safely | `swap` with empty path → `Error(Contract, #8)` = `EmptyPath` | PASS |
| Expected methods callable | `version`, `get_admin`, `is_paused`, `set_paused`, `get_protocol` invoked live | PASS |
| Events emitted live | `Initialized`, `PauseChanged` observed in tx meta | PASS |

## Verification boundary (honest scope)

- Full multi-hop `swap` execution is proven by 52 unit tests against the exact
  deployed WASM logic (same source, same `stellar contract build` profile),
  including atomicity and delivery-verification cases.
- A live testnet `swap` additionally requires a REGISTERED protocol adapter
  backed by funded testnet liquidity. No production adapters exist yet (see
  Remaining Work) — so no live swap was executed. The registry is empty by
  design; any swap attempt today fails safely with `ProtocolNotFound`.
- Interact read-only any time:
  `stellar contract invoke --id CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP --source-account <any-testnet-identity> --network testnet -- version`

## Indexer pointer (`stellariq-data`)

Subscribe to contract `CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP`
on testnet. Event schemas: `docs/events.md`.
