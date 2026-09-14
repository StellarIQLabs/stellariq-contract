#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Env, Symbol};

const COUNTER: Symbol = symbol_short!("COUNTER");

#[contracttype]
pub enum DataKey {
    Greeting,
}

#[contract]
pub struct ExampleContract;

#[contractimpl]
impl ExampleContract {
    pub fn initialize(env: Env, greeting: Symbol) {
        if env.storage().instance().has(&DataKey::Greeting) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Greeting, &greeting);
        env.storage().instance().set(&COUNTER, &0_i32);
    }

    pub fn greeting(env: Env) -> Symbol {
        env.storage()
            .instance()
            .get(&DataKey::Greeting)
            .expect("not initialized")
    }

    pub fn set_greeting(env: Env, greeting: Symbol) {
        env.storage().instance().set(&DataKey::Greeting, &greeting);
    }

    pub fn increment(env: Env) -> i32 {
        let count: i32 = env.storage().instance().get(&COUNTER).unwrap_or(0);
        let next = count + 1;
        env.storage().instance().set(&COUNTER, &next);
        next
    }

    pub fn count(env: Env) -> i32 {
        env.storage().instance().get(&COUNTER).unwrap_or(0)
    }

    pub fn hello(_env: Env, _name: Symbol) -> Symbol {
        symbol_short!("Hello")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Env, ExampleContractClient<'static>) {
        let env = Env::default();
        let contract_id = env.register(ExampleContract, ());
        let client = ExampleContractClient::new(&env, &contract_id);
        env.mock_all_auths();
        (env, client)
    }

    #[test]
    fn initialize_and_read_greeting() {
        let (_env, client) = setup();
        let greeting = symbol_short!("Soroban");
        client.initialize(&greeting);
        assert_eq!(client.greeting(), greeting);
    }

    #[test]
    fn set_greeting_updates() {
        let (_env, client) = setup();
        client.initialize(&symbol_short!("Hi"));
        let new = symbol_short!("Hey");
        client.set_greeting(&new);
        assert_eq!(client.greeting(), new);
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn double_init_fails() {
        let (_env, client) = setup();
        client.initialize(&symbol_short!("A"));
        client.initialize(&symbol_short!("B"));
    }

    #[test]
    fn counter_starts_at_zero() {
        let (_env, client) = setup();
        client.initialize(&symbol_short!("Go"));
        assert_eq!(client.count(), 0);
    }

    #[test]
    fn increment_increases_count() {
        let (_env, client) = setup();
        client.initialize(&symbol_short!("Go"));
        assert_eq!(client.increment(), 1);
        assert_eq!(client.increment(), 2);
        assert_eq!(client.count(), 2);
    }

    #[test]
    fn hello_returns_greeting() {
        let (_env, client) = setup();
        client.initialize(&symbol_short!("Go"));
        assert_eq!(
            client.hello(&symbol_short!("Stellar")),
            symbol_short!("Hello")
        );
    }
}
