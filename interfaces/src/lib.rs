#![no_std]

//! Shared StellarIQ contract-level interfaces and types.
//!
//! This crate is the single source of truth for every value that crosses a
//! contract boundary (router arguments, adapter calls, emitted events) so that
//! `stellariq-contracts`, `stellariq-app` and the `stellariq-data` indexer can
//! never disagree about shapes, error codes, or event schemas.
//!
//! ## Precision assumptions
//!
//! - All asset amounts are raw `i128` smallest-unit integers (e.g. stroops for
//!   7-decimal tokens). The contracts NEVER convert between decimals.
//! - Tokens with different decimals are passed through opaquely; quote math in
//!   `stellariq-data` MUST use raw units.
//! - Floating-point arithmetic is FORBIDDEN for financial calculations. The only
//!   arithmetic helpers here ([`checked_mul_div`]) use checked integer ops and
//!   return `None` instead of overflowing, panicking, or rounding silently.
//! - Division truncates toward zero (Rust `i128` semantics). Callers that need
//!   round-up behavior must add `denom - 1` before dividing, explicitly.
//!
//! ## Invalid states
//!
//! Types are deliberately narrow: amounts are signed because SEP-41 uses
//! `i128`, but every entrypoint rejects non-positive inputs before touching
//! state. See [`RouterError`] for the stable error codes.

use soroban_sdk::{contracterror, contractevent, contracttype, Address, BytesN, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of hops in a single route.
///
/// Bounds execution loops (gasDoS protection) and keeps multi-hop accounting
/// auditable. Routes needing more hops must be split client-side.
pub const MAX_HOPS: u32 = 5;

/// Denominator for basis-point math (10_000 bps = 100%).
pub const BPS_DENOM: i128 = 10_000;

/// Schema version of the event payloads emitted by the router.
///
/// Bumped whenever an event struct gains/loses/renames a field so the
/// `stellariq-data` indexer can branch on versions instead of guessing.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Assets / routes / execution parameters
// ---------------------------------------------------------------------------

/// One validated step of an already-computed route.
///
/// A route is produced OFF-CHAIN by `stellariq-data` and only verified
/// on-chain. Steps chain so that `steps[i].token_out == steps[i+1].token_in`,
/// `steps[0].token_in` is the swap input and the last `token_out` is the swap
/// output. The router enforces this; discontinuous routes fail safely.
///
/// `protocol` is a registry key (e.g. `"soroswap"`, `"phoenix"`, `"test"`),
/// NEVER a raw executable address — the router resolves it to an adapter
/// registered by admin. `pool` identifies the market inside that protocol and
/// is opaque to the router (forwarded to the adapter + recorded in events).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapStep {
    pub protocol: Symbol,
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    /// Minimum acceptable output of THIS hop. Enforced per hop in addition to
    /// the global swap minimum, so a bad intermediate fill can never be
    /// masked by later hops.
    pub amount_out_min: i128,
}

/// Parameters for a validated swap execution (mirrors `Router::swap` args).
///
/// Kept as a struct so `stellariq-app` and indexer code can construct /
/// decode one canonical shape.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapParams {
    pub trader: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out_min: i128,
    /// Unix timestamp (ledger close time). Execution with
    /// `ledger.timestamp() > deadline` is rejected.
    pub deadline: u64,
    pub path: Vec<SwapStep>,
}

// ---------------------------------------------------------------------------
// Errors (stable discriminants — NEVER renumber, only append)
// ---------------------------------------------------------------------------

/// Router failure modes. Every financial failure rolls back atomically.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RouterError {
    /// `initialize` called more than once.
    AlreadyInitialized = 1,
    /// Any entrypoint called before `initialize`.
    NotInitialized = 2,
    /// Caller is not the stored admin.
    NotAuthorized = 3,
    /// `swap` attempted while paused.
    Paused = 4,
    /// `deadline` is before the current ledger timestamp.
    Expired = 5,
    /// `amount_in` (or a hop input) is not strictly positive.
    ZeroAmount = 6,
    /// Global `amount_out_min` is not strictly positive (protection must
    /// always be on; callers cannot opt out of slippage checks).
    InvalidMinOut = 7,
    /// Route contains no steps.
    EmptyPath = 8,
    /// Route exceeds [`MAX_HOPS`].
    TooManyHops = 9,
    /// Hop tokens do not chain, or route endpoints mismatch the swap assets.
    DiscontinuousPath = 10,
    /// Step references a protocol with no registered adapter.
    ProtocolNotFound = 11,
    /// A hop or the final output fell below its minimum (slippage).
    InsufficientOutput = 12,
    /// Adapter returned a non-positive amount or otherwise failed.
    SwapFailed = 13,
    /// Proposed admin address is invalid.
    InvalidAdmin = 14,
    /// Checked integer arithmetic would overflow.
    Overflow = 15,
}

// ---------------------------------------------------------------------------
// Events (schema v1 — see EVENT_SCHEMA_VERSION)
// ---------------------------------------------------------------------------

/// Full settlement record for one swap. This is the PRIMARY indexer input:
/// everything `stellariq-data` needs to reconstruct swap activity lives here
/// or in the matching `hop_executed` records (joined via `execution_id`).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapExecuted {
    /// Filterable: all swaps by one trader.
    #[topic]
    pub trader: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,
    /// Ordered protocol ids traversed (mirrors the route).
    pub protocols: Vec<Symbol>,
    /// Per-contract monotonic id; unique per swap for this contract instance.
    /// Join key for `hop_executed` records.
    pub execution_id: u64,
    /// Ledger sequence at execution (timestamp context comes from Stellar).
    pub ledger: u32,
    pub version: u32,
}

/// Per-hop execution record, emitted in order with `hop_index` 0..n.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HopExecuted {
    /// Filterable: hop records for one execution.
    #[topic]
    pub execution_id: u64,
    pub trader: Address,
    pub hop_index: u32,
    pub protocol: Symbol,
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,
    pub version: u32,
}

/// Emitted once by `initialize`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
    pub version: u32,
}

/// Emitted by `set_admin`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChanged {
    #[topic]
    pub old_admin: Address,
    #[topic]
    pub new_admin: Address,
    pub version: u32,
}

/// Emitted by `set_paused` (both directions; `paused` tells which).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseChanged {
    #[topic]
    pub admin: Address,
    pub paused: bool,
    pub version: u32,
}

/// Emitted by `set_protocol`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolSet {
    #[topic]
    pub protocol: Symbol,
    pub adapter: Address,
    pub version: u32,
}

/// Emitted by `remove_protocol`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolRemoved {
    #[topic]
    pub protocol: Symbol,
    pub version: u32,
}

/// Emitted by `upgrade`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Upgraded {
    #[topic]
    pub admin: Address,
    pub wasm_hash: BytesN<32>,
    pub version: u32,
}

// ---------------------------------------------------------------------------
// Adapter interface contract
// ---------------------------------------------------------------------------

/// Narrow cross-contract interface every protocol adapter MUST implement.
///
/// The router calls ONLY this entrypoint, ONLY on adapters resolved through
/// the admin-managed protocol registry. There is intentionally no generic
/// "call anything" path: `pool` is data, `recipient` receives `token_out`,
/// and the returned `amount_out` is verified against `amount_out_min`.
///
/// Adapters MUST be non-custodial beyond the forwarded hop input and MUST NOT
/// require any authorization other than operating on their own balances.
pub const ADAPTER_SWAP_FN: &str = "swap";

// ---------------------------------------------------------------------------
// Checked integer math (no floats, no silent overflow)
// ---------------------------------------------------------------------------

/// Compute `amount * num / denom` with fully checked integer arithmetic.
///
/// Returns `None` when `denom <= 0`, any input is negative, or the
/// intermediate would overflow `i128`. Truncates toward zero on division.
///
/// Used for fixed-point rate math (e.g. test-adapter pricing); production
/// adapters SHOULD reuse it instead of hand-rolling arithmetic.
pub fn checked_mul_div(amount: i128, num: i128, denom: i128) -> Option<i128> {
    if amount < 0 || num < 0 || denom <= 0 {
        return None;
    }
    amount.checked_mul(num)?.checked_div(denom)
}

/// Compute a minimum acceptable output for `amount` with `slippage_bps`
/// basis points of tolerance: `amount * (10_000 - slippage_bps) / 10_000`.
///
/// Returns `None` when `slippage_bps > 10_000` or arithmetic overflows.
/// Pure helper for quote-side code and tests; the router itself enforces
/// caller-supplied minimums rather than computing them.
pub fn min_out_with_slippage(amount: i128, slippage_bps: i128) -> Option<i128> {
    if slippage_bps < 0 || slippage_bps > BPS_DENOM {
        return None;
    }
    checked_mul_div(amount, BPS_DENOM - slippage_bps, BPS_DENOM)
}

// ---------------------------------------------------------------------------
// Tests (pure validation logic; Env-dependent tests live in the contracts)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_exact() {
        assert_eq!(checked_mul_div(1_000, 3, 2), Some(1_500));
        assert_eq!(checked_mul_div(0, 5, 7), Some(0));
    }

    #[test]
    fn mul_div_truncates_toward_zero() {
        // 10 * 1 / 3 = 3.33… -> 3, never rounded up silently.
        assert_eq!(checked_mul_div(10, 1, 3), Some(3));
    }

    #[test]
    fn mul_div_rejects_bad_inputs() {
        assert_eq!(checked_mul_div(10, 1, 0), None);
        assert_eq!(checked_mul_div(10, 1, -3), None);
        assert_eq!(checked_mul_div(-10, 1, 3), None);
        assert_eq!(checked_mul_div(10, -1, 3), None);
    }

    #[test]
    fn mul_div_rejects_overflow() {
        assert_eq!(checked_mul_div(i128::MAX, 2, 1), None);
        // (MAX/2+1) * 2 overflows even though /2 would fit: no silent wrap.
        assert_eq!(checked_mul_div(i128::MAX / 2 + 1, 2, 2), None);
    }

    #[test]
    fn slippage_helper_bounds() {
        assert_eq!(min_out_with_slippage(10_000, 0), Some(10_000));
        assert_eq!(min_out_with_slippage(10_000, 100), Some(9_900));
        assert_eq!(min_out_with_slippage(10_000, 10_000), Some(0));
        assert_eq!(min_out_with_slippage(10_000, 10_001), None);
        assert_eq!(min_out_with_slippage(10_000, -1), None);
        assert_eq!(min_out_with_slippage(-5, 100), None);
    }

    #[test]
    fn error_discriminants_are_stable() {
        // The indexer and app decode raw codes: NEVER renumber.
        assert_eq!(RouterError::AlreadyInitialized as u32, 1);
        assert_eq!(RouterError::NotInitialized as u32, 2);
        assert_eq!(RouterError::NotAuthorized as u32, 3);
        assert_eq!(RouterError::Paused as u32, 4);
        assert_eq!(RouterError::Expired as u32, 5);
        assert_eq!(RouterError::ZeroAmount as u32, 6);
        assert_eq!(RouterError::InvalidMinOut as u32, 7);
        assert_eq!(RouterError::EmptyPath as u32, 8);
        assert_eq!(RouterError::TooManyHops as u32, 9);
        assert_eq!(RouterError::DiscontinuousPath as u32, 10);
        assert_eq!(RouterError::ProtocolNotFound as u32, 11);
        assert_eq!(RouterError::InsufficientOutput as u32, 12);
        assert_eq!(RouterError::SwapFailed as u32, 13);
        assert_eq!(RouterError::InvalidAdmin as u32, 14);
        assert_eq!(RouterError::Overflow as u32, 15);
    }

    #[test]
    fn bounds_are_sane() {
        assert!(MAX_HOPS >= 1 && MAX_HOPS <= 10);
        assert_eq!(BPS_DENOM, 10_000);
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }
}
