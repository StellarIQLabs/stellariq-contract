#!/usr/bin/env bash
#
# Deploy the StellarIQ router to Stellar.
#
#   ./scripts/deploy.sh [testnet]            # default: testnet
#   ./scripts/deploy.sh mainnet --confirm-mainnet
#
# Only the ROUTER is ever deployed. The test adapter is test-only and this
# script refuses to deploy anything else.
#
# Required environment (see .env.example):
#   STELLAR_SOURCE_ACCOUNT  a `stellar keys` identity name (NOT a secret key)
#   ADMIN_ADDRESS            the Stellar address that will own the router
#
# The script: validates env -> validates network/identity -> builds -> deploys
# -> initializes atomically -> records metadata -> verifies. Any failure
# aborts before the next step (set -euo pipefail).
#
# Secrets policy: secret keys live ONLY in `stellar keys` identities (OS
# keyring / config dir). They are never read, printed, or written by this
# script, and never land in deployments/ metadata.
set -euo pipefail

NETWORK="${1:-testnet}"
CONFIRM="${2:-}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOYMENTS_DIR="$REPO_ROOT/deployments"
WASM="$REPO_ROOT/target/wasm32v1-none/release/stellariq_router.wasm"

# Load .env if present (non-secret defaults only; never commit real .env).
if [ -f "$REPO_ROOT/.env" ]; then
  # shellcheck disable=SC1091
  set -a; source "$REPO_ROOT/.env"; set +a
fi

log()  { echo "==> $*"; }
fail() { echo "ERROR: $*" >&2; exit 1; }

# --- 1. Validate environment -------------------------------------------------
command -v stellar >/dev/null 2>&1 || fail "stellar CLI not found in PATH"
command -v cargo >/dev/null 2>&1   || fail "cargo not found in PATH (needed for build)"

SOURCE="${STELLAR_SOURCE_ACCOUNT:-}"
ADMIN="${ADMIN_ADDRESS:-}"
[ -n "$SOURCE" ] || fail "STELLAR_SOURCE_ACCOUNT is not set (see .env.example)"
[ -n "$ADMIN" ]  || fail "ADMIN_ADDRESS is not set (see .env.example)"

# --- 2. Validate network ------------------------------------------------------
case "$NETWORK" in
  testnet|mainnet) ;;
  *) fail "unknown network '$NETWORK' (expected testnet|mainnet)" ;;
esac
if [ "$NETWORK" = "mainnet" ] && [ "$CONFIRM" != "--confirm-mainnet" ]; then
  fail "mainnet deploys require an explicit checkpoint: rerun with 'mainnet --confirm-mainnet'"
fi

# --- 3. Validate identity ------------------------------------------------------
if ! stellar keys address "$SOURCE" >/dev/null 2>&1; then
  fail "identity '$SOURCE' not found. Create it first: stellar keys generate $SOURCE --network $NETWORK"
fi
log "network=$NETWORK source_identity=$SOURCE admin=$ADMIN"

# --- 4. Build ------------------------------------------------------------------
log "building contracts (stellar contract build)"
(
  cd "$REPO_ROOT"
  stellar contract build
)
[ -f "$WASM" ] || fail "expected wasm artifact missing: $WASM"
WASM_HASH="$(sha256sum "$WASM" | cut -d' ' -f1)"
log "wasm sha256=$WASM_HASH"

# --- 5. Deploy (router only, by construction) -----------------------------------
log "deploying router"
DEPLOY_OUT="$(stellar contract deploy \
  --wasm "$WASM" \
  --source-account "$SOURCE" \
  --network "$NETWORK")"
CONTRACT_ID="$(echo "$DEPLOY_OUT" | grep -Eo 'C[A-Z0-9]{55}' | tail -n 1)"
[ -n "$CONTRACT_ID" ] || fail "could not parse contract id from deploy output: $DEPLOY_OUT"
log "router contract id: $CONTRACT_ID"

# --- 6. Initialize atomically (permissionless one-shot: must follow deploy) -----
log "initializing router (admin=$ADMIN)"
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- initialize --admin "$ADMIN" >/dev/null
log "initialized"

# --- 7. Record deployment metadata (no secrets, ever) ---------------------------
mkdir -p "$DEPLOYMENTS_DIR"
COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
VERSION="$(grep '^version' "$REPO_ROOT/contracts/router/Cargo.toml" | head -n 1 | cut -d'"' -f2)"
STAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
META="$DEPLOYMENTS_DIR/$NETWORK-router.json"
cat > "$META" <<EOF
{
  "network": "$NETWORK",
  "contract": "router",
  "contractId": "$CONTRACT_ID",
  "wasmSha256": "$WASM_HASH",
  "deployedAt": "$STAMP",
  "commit": "$COMMIT",
  "version": "$VERSION",
  "admin": "$ADMIN",
  "deployerIdentity": "$SOURCE"
}
EOF
log "metadata written to $META"

# --- 8. Verify ------------------------------------------------------------------
log "running post-deploy verification"
"$REPO_ROOT/scripts/verify.sh" "$NETWORK" "$CONTRACT_ID"
log "DEPLOY OK: $CONTRACT_ID on $NETWORK"
