# StellarIQ Contracts (`stellariq-contracts`)

On-chain execution layer for StellarIQ: a minimal, audited Soroban swap router
with safe multi-hop execution, slippage/deadline protection, and
indexer-friendly events.

- Architecture: [`docs/architecture.md`](docs/architecture.md)
- App integration: [`docs/integration.md`](docs/integration.md) (Task 14)
- Security review: [`docs/security-review.md`](docs/security-review.md) (Task 7)
- Testnet deployment: [`docs/testnet-deployment.md`](docs/testnet-deployment.md) (Task 13)

## Layout

```
contracts/router/        # the only deployed contract: swap router
contracts/test-adapter/  # TEST-ONLY protocol adapter (never deployed)
interfaces/              # shared contract types (assets, routes, errors, events)
scripts/                 # deployment / verification tooling (Task 10)
deployments/             # deployment metadata per network (Task 12)
docs/                    # architecture, events, security, integration
```

Flow: `stellariq-app` → quote from `stellariq-data` → build tx → wallet signs →
Soroban executes → `stellariq-data` indexes events. Contracts never depend on
any centralized API at execution time.

## Toolchain

| Tool | Version (verified) |
|---|---|
| Rust | 1.98.1 stable (minimum 1.84 for `wasm32v1-none`) |
| `stellar-cli` | 28.0.0 |
| `soroban-sdk` | 27 (see `Cargo.toml`) |
| Build target | `wasm32v1-none` (via `stellar contract build`, never bare `cargo build` for wasm) |
| Network | testnet first; mainnet only with explicit confirmation |

## Quickstart

```sh
# 1. Toolchain
rustup target add wasm32v1-none
# install stellar-cli: https://developers.stellar.org/docs/build/smart-contracts/getting-started/setup

# 2. Format / build / test
cargo fmt --all
stellar contract build
cargo test

# 3. Deploy (testnet)
cp .env.example .env   # then set STELLAR_SOURCE_ACCOUNT to a funded testnet identity
./scripts/deploy.sh testnet
./scripts/verify.sh testnet
```

No secrets are ever committed. See `.env.example` and `docs/` before deploying.
