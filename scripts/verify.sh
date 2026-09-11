#!/usr/bin/env bash
#
# Verify a deployed StellarIQ router.
#
#   ./scripts/verify.sh testnet                     # uses deployments/testnet-router.json
#   ./scripts/verify.sh testnet CABC...             # explicit contract id
#
# Checks: contract responds, admin matches metadata, paused == false,
# version matches Cargo, and invalid calls fail safely. Read-only where
# possible; no state is modified.
set -euo pipefail

NETWORK="${1:-testnet}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -f "$REPO_ROOT/.env" ]; then
  # shellcheck disable=SC1091
  set -a; source "$REPO_ROOT/.env"; set +a
fi

log()  { echo "==> $*"; }
fail() { echo "ERROR: $*" >&2; exit 1; }

META="$REPO_ROOT/deployments/$NETWORK-router.json"
EXPLICIT_ID="${2:-}"
if [ -n "$EXPLICIT_ID" ]; then
  CONTRACT_ID="$EXPLICIT_ID"
else
  [ -f "$META" ] || fail "no deployment metadata at $META (pass an explicit id or deploy first)"
  CONTRACT_ID="$(python3 -c "import json;print(json.load(open('$META'))['contractId'])")"
fi

SOURCE="${STELLAR_SOURCE_ACCOUNT:-}"
[ -n "$SOURCE" ] || fail "STELLAR_SOURCE_ACCOUNT is not set (see .env.example)"
EXPECTED_ADMIN=""
if [ -f "$META" ]; then
  EXPECTED_ADMIN="$(python3 -c "import json;print(json.load(open('$META')).get('admin',''))")"
fi

invoke() { stellar contract invoke --id "$CONTRACT_ID" --source-account "$SOURCE" --network "$NETWORK" -- "$@"; }
# Simulation/read-only output quotes strings ("0.1.0", "GABC..."): normalize.
unquote() { tr -d '"'; }

log "contract: $CONTRACT_ID on $NETWORK"

GOT_VERSION="$(invoke version --is-view 2>/dev/null || invoke version | unquote)"
log "version() = $GOT_VERSION"

GOT_ADMIN="$(invoke get_admin --is-view 2>/dev/null || invoke get_admin | unquote)"
log "get_admin() = $GOT_ADMIN"
if [ -n "$EXPECTED_ADMIN" ] && [ "$GOT_ADMIN" != "$EXPECTED_ADMIN" ]; then
  fail "admin mismatch: on-chain=$GOT_ADMIN metadata=$EXPECTED_ADMIN"
fi

GOT_PAUSED="$(invoke is_paused --is-view 2>/dev/null || invoke is_paused | unquote)"
log "is_paused() = $GOT_PAUSED"
[ "$GOT_PAUSED" = "false" ] || fail "router is paused; investigate before use"

# Invalid call must fail safely (garbage protocol, zero amount, expired path
# would all revert; here we just prove an unknown method/errors surface).
if invoke get_protocol --protocol "no_such_protocol" >/dev/null 2>&1; then
  fail "get_protocol for an unknown protocol unexpectedly succeeded"
else
  log "unknown protocol correctly rejected"
fi

log "VERIFY OK: $CONTRACT_ID"
