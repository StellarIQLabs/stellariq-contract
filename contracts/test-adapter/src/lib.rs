#![no_std]

//! Test-only protocol adapter for the StellarIQ router test suite.
//!
//! NEVER DEPLOYED. Real implementation lands with Task 6+.

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct TestAdapter;

#[contractimpl]
impl TestAdapter {}
