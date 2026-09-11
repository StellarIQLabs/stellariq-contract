#![no_std]

//! StellarIQ swap router.
//!
//! Full implementation lands in Tasks 4–6 (shared types, router, execution).
//! This skeleton only guarantees the workspace shape compiles.

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct Router;

#[contractimpl]
impl Router {}
