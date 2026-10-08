# StellarIQ Contracts

Soroban smart contracts for **StellarIQ Give**: transparent charity
donations on Stellar. Donors give in any Stellar asset, charities receive the
token they asked for, and every donation leaves a public on-chain receipt.

| Contract | Purpose | Testnet |
|---|---|---|
| `donations` | Charity campaigns, direct-to-beneficiary donations, receipts | [`CBCHKIDR...F2KX`](https://stellar.expert/explorer/testnet/contract/CBCHKIDRFJ4KO2DGJEP75NJPYN65YVD6QOVVHC5IU7PRTHGHW75OF2KX) |
| `router` | Multi-hop swap router that converts a donor's asset into the campaign token | [`CC277AA6...VHSP`](https://stellar.expert/explorer/testnet/contract/CC277AA6E6WZIQRA4N45TQ3O6VV5MUSDMRZCNHO43QENMYXV6E5OVHSP) |

- Donations: [`docs/donations.md`](docs/donations.md)
- Architecture: [`docs/architecture.md`](docs/architecture.md)
- App integration: [`docs/integration.md`](docs/integration.md)
- Security review: [`docs/security-review.md`](docs/security-review.md)
- Testnet deployment: [`docs/testnet-deployment.md`](docs/testnet-deployment.md)

## Layout

```
contracts/donations/     # charity campaigns and donation receipts
contracts/router/        # swap router used to convert donations
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
cp .env.example .env   # then set STELLAR_SOURCE_ACCOUNT + ADMIN_ADDRESS
./scripts/deploy.sh testnet
./scripts/verify.sh testnet
```

No secrets are ever committed. See `.env.example` and `docs/` before deploying.
