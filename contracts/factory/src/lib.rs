#![no_std]
use emergency_guard::{EmergencyGuard, GuardError};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal,
};

const PAUSE_CREATE_PAIR_FLAG: u32 = 1 << 6;

/// Storage key for pair registry.
/// Stored in **instance** storage because the factory is a singleton contract
/// and pair mappings are global state that should share the contract's TTL.
/// Using instance storage avoids per-entry persistent rent and reduces the
/// ledger footprint to a single entry per invocation.
#[contracttype]
pub enum DataKey {
    Pair(Address, Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
    PairAlreadyExists = 5,
    InvalidThreshold = 6,
}

#[contract]
pub struct LiquidityPoolFactory;

fn check_not_paused(env: &Env) -> Result<(), Error> {
    if EmergencyGuard::is_paused(env.clone(), PAUSE_CREATE_PAIR_FLAG) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

#[contractimpl]
impl LiquidityPoolFactory {
    /// Initialize the factory's emergency guard state with a set of admins.
    pub fn initialize(env: Env, admins: soroban_sdk::Vec<Address>, threshold: u32) -> Result<(), Error> {
        EmergencyGuard::initialize(env.clone(), admins, threshold).map_err(|e| match e {
            GuardError::AlreadyInitialized => Error::AlreadyInitialized,
            GuardError::InvalidThreshold => Error::InvalidThreshold,
            _ => Error::Unauthorized,
        })
    }

    /// Deploys a new Liquidity Pool contract for a unique pair of tokens.
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Result<Address, Error> {
        check_not_paused(&env)?;

        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // Instance storage: cheaper rent, no per-entry TTL management.
        if env
            .storage()
            .instance()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            return Err(Error::PairAlreadyExists);
        }

        let salt = env
            .crypto()
            .sha256(&(token_0.clone(), token_1.clone()).to_xdr(&env));

        let deployed_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, soroban_sdk::Vec::<soroban_sdk::Val>::new(&env));

        let init_args = soroban_sdk::vec![
            &env,
            env.current_contract_address().into_val(&env),
            token_0.clone().into_val(&env),
            token_1.clone().into_val(&env)
        ];

        let _res: soroban_sdk::Val = env.invoke_contract(
            &deployed_address,
            &soroban_sdk::Symbol::new(&env, "initialize"),
            init_args,
        );

        // One instance write instead of one persistent write.
        env.storage()
            .instance()
        Ok(deployed_address)y::Pair(token_0, token_1), &deployed_address);

        deployed_address
    }

    /// Returns the pool address for the given token pair, if it exists.
    pub fn get_pair(env: Env, token_a: Address, token_b: Address) -> Option<Address> {
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // One instance read instead of one persistent read.
        env.storage()
            .instance()
            .get(&DataKey::Pair(token_0, token_1))
    }

    /// Admin-only: pause or unpause pair creation.
    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, PAUSE_CREATE_PAIR_FLAG, paused).map_err(|e| match e {
            GuardError::Unauthorized => Error::Unauthorized,
            _ => Error::Unauthorized,
        })
    }

    /// Admin-only: emergency pause all factory operations.
    pub fn emergency_pause(env: Env, approvers: soroban_sdk::Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(env, approvers).map_err(|e| match e {
            GuardError::NotInitialized => Error::NotInitialized,
            GuardError::InsufficientSignatures => Error::Unauthorized,
            _ => Error::Unauthorized,
        })
    }

    /// Read the factory's current pause state.
    pub fn get_pause_state(env: Env) -> u32 {
        EmergencyGuard::get_pause_state(env)
    }
}

mod test;
