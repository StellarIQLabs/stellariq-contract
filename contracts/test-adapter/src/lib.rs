#![no_std]

//! Test-only protocol adapter for the StellarIQ router test suite.
//!
//! NEVER DEPLOYED. Implements the shared [`Adapter`] interface with
//! deterministic fixed-point pricing (`out = in * num / denom` per pool) so
//! router tests exercise the real cross-contract execution path without any
//! live DEX dependency.
//!
//! [`Adapter`]: stellariq_interfaces::Adapter

use soroban_sdk::{contract, contractimpl, contracttype, token::TokenClient, Address, Env};
use stellariq_interfaces::{checked_mul_div, AdapterError};

#[contracttype]
#[derive(Clone)]
enum AdapterKey {
    Admin,
    /// pool -> (rate numerator, rate denominator)
    Rate(Address),
    /// pool -> injected failure flag (simulates a venue rejecting execution)
    Fail(Address),
    /// pool -> overridden REPORTED output (delivery stays honest).
    /// Simulates a misreporting adapter: the router must fail closed on
    /// inflated reports and settle on verified delivery for deflated ones.
    Misreport(Address),
}

#[contract]
pub struct TestAdapter;

#[contractimpl]
impl TestAdapter {
    pub fn initialize(env: Env, admin: Address) -> Result<(), AdapterError> {
        if env.storage().instance().has(&AdapterKey::Admin) {
            return Err(AdapterError::AlreadyInitialized);
        }
        env.storage().instance().set(&AdapterKey::Admin, &admin);
        Ok(())
    }

    pub fn set_rate(
        env: Env,
        admin: Address,
        pool: Address,
        num: i128,
        denom: i128,
    ) -> Result<(), AdapterError> {
        Self::require_admin(&env, &admin)?;
        if num < 0 || denom <= 0 {
            return Err(AdapterError::InvalidRate);
        }
        env.storage()
            .instance()
            .set(&AdapterKey::Rate(pool), &(num, denom));
        Ok(())
    }

    pub fn set_fail(
        env: Env,
        admin: Address,
        pool: Address,
        fail: bool,
    ) -> Result<(), AdapterError> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&AdapterKey::Fail(pool), &fail);
        Ok(())
    }

    pub fn set_misreport(
        env: Env,
        admin: Address,
        pool: Address,
        reported: i128,
    ) -> Result<(), AdapterError> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&AdapterKey::Misreport(pool), &reported);
        Ok(())
    }

    pub fn clear_misreport(env: Env, admin: Address, pool: Address) -> Result<(), AdapterError> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .remove(&AdapterKey::Misreport(pool));
        Ok(())
    }

    /// Deterministic priced swap. Delivers `token_out` from own inventory.
    ///
    /// Minimum enforcement is intentionally LEFT to the router: this adapter
    /// reports honestly so the router's own slippage checks are what's under
    /// test. (Production adapters typically enforce venue-side too; the
    /// router never relies on that.)
    #[allow(clippy::too_many_arguments)]
    pub fn swap(
        env: Env,
        pool: Address,
        token_in: Address,
        token_out: Address,
        amount_in: i128,
        amount_out_min: i128,
        recipient: Address,
    ) -> Result<i128, AdapterError> {
        // No `require_auth`: called by the router, operating on own balances.
        Self::initialized(&env)?;
        let _ = amount_out_min; // router-enforced; see module docs
        if amount_in <= 0 {
            return Err(AdapterError::Failed);
        }
        if env
            .storage()
            .instance()
            .get(&AdapterKey::Fail(pool.clone()))
            .unwrap_or(false)
        {
            return Err(AdapterError::Failed);
        }
        let me = env.current_contract_address();
        // Sanity: the router must have forwarded the hop input beforehand.
        if TokenClient::new(&env, &token_in).balance(&me) < amount_in {
            return Err(AdapterError::Failed);
        }
        let (num, denom): (i128, i128) = env
            .storage()
            .instance()
            .get(&AdapterKey::Rate(pool.clone()))
            .unwrap_or((1, 1)); // default: 1:1
        let out = checked_mul_div(amount_in, num, denom).ok_or(AdapterError::Failed)?;
        // Insufficient inventory traps in the token contract; the router maps
        // every adapter failure to `SwapFailed`.
        TokenClient::new(&env, &token_out).transfer(&me, &recipient, &out);
        // Delivery above is always honest; only the REPORT may lie.
        Ok(env
            .storage()
            .instance()
            .get(&AdapterKey::Misreport(pool))
            .unwrap_or(out))
    }

    // -- private ----------------------------------------------------------

    fn initialized(env: &Env) -> Result<(), AdapterError> {
        if env.storage().instance().has(&AdapterKey::Admin) {
            Ok(())
        } else {
            Err(AdapterError::NotInitialized)
        }
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), AdapterError> {
        let stored: Address = env
            .storage()
            .instance()
            .get(&AdapterKey::Admin)
            .ok_or(AdapterError::NotInitialized)?;
        if admin != &stored {
            return Err(AdapterError::NotAuthorized);
        }
        admin.require_auth();
        Ok(())
    }
}
