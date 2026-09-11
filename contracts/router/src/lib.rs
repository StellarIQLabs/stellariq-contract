#![no_std]

//! StellarIQ swap router — the only deployed StellarIQ contract.
//!
//! Executes pre-computed multi-hop routes with strict on-chain guarantees:
//! trader authorization, deadline enforcement, per-hop + global slippage
//! protection, ordered execution through registered protocol adapters, and
//! atomic failure (any violation rolls back the whole swap).
//!
//! What this contract deliberately does NOT do: price discovery (belongs to
//! `stellariq-data`), custody (balances are transient within one transaction),
//! arbitrary external calls (only SEP-41 `transfer` + the fixed adapter
//! `swap` entrypoint on registered adapters), or protocol fees (deferred).

use soroban_sdk::{
    contract, contractimpl, contracttype, token::TokenClient, Address, BytesN, Env, String, Symbol,
    Vec,
};
use stellariq_interfaces::{
    AdapterClient, AdminChanged, HopExecuted, Initialized, PauseChanged, ProtocolRemoved,
    ProtocolSet, RouterError, SwapExecuted, SwapStep, Upgraded, EVENT_SCHEMA_VERSION, MAX_HOPS,
};

#[cfg(test)]
mod test;

/// Storage layout.
///
/// `Admin` / `Paused` / `Nonce` are small, always read together, and live in
/// instance storage. The protocol registry is potentially multi-entry and
/// lives in persistent storage so entries survive independently and can carry
/// their own TTL.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Paused,
    Nonce,
    Protocol(Symbol),
}

/// TTL policy for storage entries (ledgers).
const TTL_THRESHOLD: u32 = 100;
const TTL_EXTEND: u32 = 10_000;

#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    // -- Lifecycle --------------------------------------------------------

    /// One-shot initialization. Binds the admin; zeroes pause + nonce.
    pub fn initialize(env: Env, admin: Address) -> Result<(), RouterError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(RouterError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::Nonce, &0u64);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
        Initialized {
            admin,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Rotate admin. The CURRENT admin authorizes; rotating onto the same
    /// address is rejected as a no-op (`InvalidAdmin`).
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), RouterError> {
        let old_admin = read_admin(&env)?;
        old_admin.require_auth();
        if new_admin == old_admin {
            return Err(RouterError::InvalidAdmin);
        }
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
        AdminChanged {
            old_admin,
            new_admin,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Circuit breaker. Halts `swap` only; admin functions stay live so a
    /// paused contract can still be managed (and unpaused).
    pub fn set_paused(env: Env, paused: bool) -> Result<(), RouterError> {
        let admin = read_admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
        PauseChanged {
            admin,
            paused,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    // -- Protocol registry -------------------------------------------------

    /// Register (or replace) the adapter contract for a protocol id.
    pub fn set_protocol(env: Env, protocol: Symbol, adapter: Address) -> Result<(), RouterError> {
        let admin = read_admin(&env)?;
        admin.require_auth();
        let key = DataKey::Protocol(protocol.clone());
        env.storage().persistent().set(&key, &adapter);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND);
        ProtocolSet {
            protocol,
            adapter,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Unregister a protocol (emergency containment for a faulty adapter).
    pub fn remove_protocol(env: Env, protocol: Symbol) -> Result<(), RouterError> {
        let admin = read_admin(&env)?;
        admin.require_auth();
        let key = DataKey::Protocol(protocol.clone());
        if !env.storage().persistent().has(&key) {
            return Err(RouterError::ProtocolNotFound);
        }
        env.storage().persistent().remove(&key);
        ProtocolRemoved {
            protocol,
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        Ok(())
    }

    /// Replace contract code. Admin-gated; storage layout must be preserved
    /// by the new code (any breaking change needs a migration plan first).
    pub fn upgrade(env: Env, wasm_hash: BytesN<32>) -> Result<(), RouterError> {
        let admin = read_admin(&env)?;
        admin.require_auth();
        Upgraded {
            admin,
            wasm_hash: wasm_hash.clone(),
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);
        env.deployer().update_current_contract_wasm(wasm_hash);
        Ok(())
    }

    // -- Read-only views ----------------------------------------------------

    /// Pinned contract version (mirrors `CARGO_PKG_VERSION`, single source).
    /// A `String` (not `Symbol`) because semver contains `.`, which `Symbol`
    /// does not permit.
    pub fn version(env: Env) -> String {
        String::from_str(&env, env!("CARGO_PKG_VERSION"))
    }

    pub fn get_admin(env: Env) -> Result<Address, RouterError> {
        read_admin(&env)
    }

    pub fn is_paused(env: Env) -> Result<bool, RouterError> {
        read_admin(&env)?; // existence check: uninitialized contracts report it
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false))
    }

    pub fn get_protocol(env: Env, protocol: Symbol) -> Result<Address, RouterError> {
        read_admin(&env)?; // existence check
        env.storage()
            .persistent()
            .get(&DataKey::Protocol(protocol))
            .ok_or(RouterError::ProtocolNotFound)
    }

    // -- Execution ----------------------------------------------------------

    /// Execute a pre-computed route. Returns the final output amount.
    ///
    /// Guarantees (each a reverted transaction on violation):
    /// * caller proved trader authorization for THESE exact arguments;
    /// * `deadline` not passed; amounts strictly positive with protection on;
    /// * route non-empty, bounded, continuous, endpoints matching;
    /// * every hop through a registered adapter, in order, each meeting its
    ///   minimum; final output meeting the global minimum;
    /// * full settlement record in events for the `stellariq-data` indexer.
    #[allow(clippy::too_many_arguments)]
    pub fn swap(
        env: Env,
        trader: Address,
        token_in: Address,
        token_out: Address,
        amount_in: i128,
        amount_out_min: i128,
        deadline: u64,
        path: Vec<SwapStep>,
    ) -> Result<i128, RouterError> {
        read_admin(&env)?; // fail fast before touching auth when uninitialized
        if env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(RouterError::Paused);
        }
        trader.require_auth();

        if amount_in <= 0 {
            return Err(RouterError::ZeroAmount);
        }
        if amount_out_min <= 0 {
            // Slippage protection is mandatory, never opt-out.
            return Err(RouterError::InvalidMinOut);
        }
        if env.ledger().timestamp() > deadline {
            return Err(RouterError::Expired);
        }

        let hops = path.len();
        if hops == 0 {
            return Err(RouterError::EmptyPath);
        }
        if hops > MAX_HOPS {
            return Err(RouterError::TooManyHops);
        }
        validate_path(&token_in, &token_out, &path)?;

        // Deterministic pre-check so insufficient funds surface as a stable
        // error code instead of a raw token-contract trap.
        if TokenClient::new(&env, &token_in).balance(&trader) < amount_in {
            return Err(RouterError::InsufficientBalance);
        }

        let router = env.current_contract_address();
        TokenClient::new(&env, &token_in).transfer(&trader, &router, &amount_in);

        let last = hops.checked_sub(1).ok_or(RouterError::EmptyPath)?;
        let mut protocols: Vec<Symbol> = Vec::new(&env);
        let mut hop_amount_in = amount_in;

        for i in 0..hops {
            let step = path.get(i).ok_or(RouterError::EmptyPath)?;
            let adapter: Address = env
                .storage()
                .persistent()
                .get(&DataKey::Protocol(step.protocol.clone()))
                .ok_or(RouterError::ProtocolNotFound)?;

            // Forward hop input to the adapter; it delivers output onward.
            // Funds are transient: router never retains a balance past the tx.
            TokenClient::new(&env, &step.token_in).transfer(&router, &adapter, &hop_amount_in);

            let recipient = if i == last {
                trader.clone()
            } else {
                router.clone()
            };
            let hop_out = AdapterClient::new(&env, &adapter)
                .try_swap(
                    &step.pool,
                    &step.token_in,
                    &step.token_out,
                    &hop_amount_in,
                    &step.amount_out_min,
                    &recipient,
                )
                .map_err(|_| RouterError::SwapFailed)?
                .map_err(|_| RouterError::SwapFailed)?;
            if hop_out <= 0 {
                return Err(RouterError::SwapFailed);
            }
            if hop_out < step.amount_out_min {
                return Err(RouterError::InsufficientOutput);
            }

            HopExecuted {
                execution_id: peek_nonce(&env)?,
                trader: trader.clone(),
                hop_index: i,
                protocol: step.protocol.clone(),
                pool: step.pool.clone(),
                token_in: step.token_in.clone(),
                token_out: step.token_out.clone(),
                amount_in: hop_amount_in,
                amount_out: hop_out,
                version: EVENT_SCHEMA_VERSION,
            }
            .publish(&env);

            protocols.push_back(step.protocol.clone());
            hop_amount_in = hop_out;
        }

        if hop_amount_in < amount_out_min {
            return Err(RouterError::InsufficientOutput);
        }

        let execution_id = next_nonce(&env)?;
        SwapExecuted {
            trader: trader.clone(),
            token_in: token_in.clone(),
            token_out: token_out.clone(),
            amount_in,
            amount_out: hop_amount_in,
            protocols,
            execution_id,
            ledger: env.ledger().sequence(),
            version: EVENT_SCHEMA_VERSION,
        }
        .publish(&env);

        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
        Ok(hop_amount_in)
    }
}

// -- Private helpers (not contract entrypoints) ------------------------------

fn read_admin(env: &Env) -> Result<Address, RouterError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(RouterError::NotInitialized)
}

/// Route shape validation: endpoint match, hop chaining, sane hop minimums.
///
/// Protocol existence is checked at execution time (per hop) so validation
/// stays a pure shape check with no storage reads.
fn validate_path(
    token_in: &Address,
    token_out: &Address,
    path: &Vec<SwapStep>,
) -> Result<(), RouterError> {
    let mut cursor = token_in.clone();
    for i in 0..path.len() {
        let step = path.get(i).ok_or(RouterError::EmptyPath)?;
        if step.token_in != cursor {
            return Err(RouterError::DiscontinuousPath);
        }
        if step.amount_out_min < 0 {
            return Err(RouterError::InvalidMinOut);
        }
        cursor = step.token_out.clone();
    }
    if &cursor != token_out {
        return Err(RouterError::DiscontinuousPath);
    }
    Ok(())
}

fn peek_nonce(env: &Env) -> Result<u64, RouterError> {
    read_admin(env)?; // keep nonce reads gated on initialization
    Ok(env
        .storage()
        .instance()
        .get(&DataKey::Nonce)
        .unwrap_or(0u64))
}

fn next_nonce(env: &Env) -> Result<u64, RouterError> {
    let next = peek_nonce(env)?
        .checked_add(1)
        .ok_or(RouterError::Overflow)?;
    env.storage().instance().set(&DataKey::Nonce, &next);
    Ok(next)
}
