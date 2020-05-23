use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;

use crate::state::{config, config_read, State};
use crate::msg::{AllowanceResponse, BalanceResponse, HandleMsg, InitMsg, QueryMsg};
use cosmwasm_std::{
    generic_err, log, to_binary, to_vec, Api, Binary, CanonicalAddr, Env, Extern, HandleResponse,
    HumanAddr, InitResponse, Querier, ReadonlyStorage, StdResult, Storage, HandleResult
};
use cosmwasm_storage::{PrefixedStorage, ReadonlyPrefixedStorage};

#[derive(Serialize, Debug, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Constants {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

pub const PREFIX_CONFIG: &[u8] = b"config";
pub const PREFIX_BALANCES: &[u8] = b"balances";
pub const PREFIX_ALLOWANCES: &[u8] = b"allowances";

pub const KEY_CONSTANTS: &[u8] = b"constants";
pub const KEY_TOTAL_SUPPLY: &[u8] = b"total_supply";

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let mut total_supply: u128 = 0;
    {
        // Initial balances
        let mut balances_store = PrefixedStorage::new(PREFIX_BALANCES, &mut deps.storage);
        for row in msg.initial_balances {
            let raw_address = deps.api.canonical_address(&row.address)?;
            let amount_raw = parse_u128(&row.amount)?;
            balances_store.set(raw_address.as_slice(), &amount_raw.to_be_bytes())?;
            total_supply += amount_raw;
        }
    }

    // Check name, symbol, decimals
    if !is_valid_name(&msg.name) {
        return Err(generic_err(
            "Name is not in the expected format (3-30 UTF-8 bytes)",
        ));
    }
    if !is_valid_symbol(&msg.symbol) {
        return Err(generic_err(
            "Ticker symbol is not in expected format [A-Z]{3,6}",
        ));
    }
    if msg.decimals > 18 {
        return Err(generic_err("Decimals must not exceed 18"));
    }

    let mut config_store = PrefixedStorage::new(PREFIX_CONFIG, &mut deps.storage);
    let constants = to_vec(&Constants {
        name: msg.name,
        symbol: msg.symbol,
        decimals: msg.decimals,
    })?;
    config_store.set(KEY_CONSTANTS, &constants)?;
    config_store.set(KEY_TOTAL_SUPPLY, &total_supply.to_be_bytes())?;
    
    let state = State {
        minter: _env.message.sender.clone(),
    };
    config(&mut deps.storage).save(&state)?;
    
    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    let state = config_read(&deps.storage).load()?;

    match msg {
        HandleMsg::Approve { spender, amount } => try_approve(deps, env, &spender, &amount),
        HandleMsg::Transfer { recipient, amount } => try_transfer(deps, env, &recipient, &amount),
        HandleMsg::TransferFrom {
            owner,
            recipient,
            amount,
        } => try_transfer_from(deps, env, &owner, &recipient, &amount),
        HandleMsg::Burn { amount } => try_burn(deps, env, &amount),
        HandleMsg::Mint { recipient, amount } => try_mint(deps, env, state, &recipient, &amount),
    }
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Balance { address } => {
            let address_key = deps.api.canonical_address(&address)?;
            let balance = read_balance(&deps.storage, &address_key)?;
            let out = to_binary(&BalanceResponse {
                balance: balance.to_string(),
            })?;
            Ok(out)
        }
        QueryMsg::Allowance { owner, spender } => {
            let owner_key = deps.api.canonical_address(&owner)?;
            let spender_key = deps.api.canonical_address(&spender)?;
            let allowance = read_allowance(&deps.storage, &owner_key, &spender_key)?;
            let out = to_binary(&AllowanceResponse {
                allowance: allowance.to_string(),
            })?;
            Ok(out)
        }
    }
}

fn try_transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: &HumanAddr,
    amount: &str,
) -> StdResult<HandleResponse> {
    let sender_address_raw = &env.message.sender;
    let recipient_address_raw = deps.api.canonical_address(recipient)?;
    let amount_raw = parse_u128(amount)?;

    perform_transfer(
        &mut deps.storage,
        &sender_address_raw,
        &recipient_address_raw,
        amount_raw,
    )?;

    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "transfer"),
            log(
                "sender",
                deps.api.human_address(&env.message.sender)?.as_str(),
            ),
            log("recipient", recipient.as_str()),
        ],
        data: None,
    };
    Ok(res)
}

fn try_transfer_from<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: &HumanAddr,
    recipient: &HumanAddr,
    amount: &str,
) -> StdResult<HandleResponse> {
    let spender_address_raw = &env.message.sender;
    let owner_address_raw = deps.api.canonical_address(owner)?;
    let recipient_address_raw = deps.api.canonical_address(recipient)?;
    let amount_raw = parse_u128(amount)?;

    let mut allowance = read_allowance(&deps.storage, &owner_address_raw, &spender_address_raw)?;
    if allowance < amount_raw {
        return Err(generic_err(format!(
            "Insufficient allowance: allowance={}, required={}",
            allowance, amount_raw
        )));
    }
    allowance -= amount_raw;
    write_allowance(
        &mut deps.storage,
        &owner_address_raw,
        &spender_address_raw,
        allowance,
    )?;
    perform_transfer(
        &mut deps.storage,
        &owner_address_raw,
        &recipient_address_raw,
        amount_raw,
    )?;

    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "transfer_from"),
            log(
                "spender",
                deps.api.human_address(&env.message.sender)?.as_str(),
            ),
            log("sender", owner.as_str()),
            log("recipient", recipient.as_str()),
        ],
        data: None,
    };
    Ok(res)
}

fn try_approve<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    spender: &HumanAddr,
    amount: &str,
) -> StdResult<HandleResponse> {
    let owner_address_raw = &env.message.sender;
    let spender_address_raw = deps.api.canonical_address(spender)?;
    let amount_raw = parse_u128(amount)?;
    write_allowance(
        &mut deps.storage,
        &owner_address_raw,
        &spender_address_raw,
        amount_raw,
    )?;
    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "approve"),
            log(
                "owner",
                deps.api.human_address(&env.message.sender)?.as_str(),
            ),
            log("spender", spender.as_str()),
        ],
        data: None,
    };
    Ok(res)
}

/// Burn tokens
///
/// Remove `amount` tokens from the system irreversibly, from signer account
///
/// @param amount the amount of money to burn
fn try_burn<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: &str,
) -> StdResult<HandleResponse> {
    let owner_address_raw = &env.message.sender;
    let amount_raw = parse_u128(amount)?;

    let mut account_balance = read_balance(&deps.storage, owner_address_raw)?;

    if account_balance < amount_raw {
        return Err(generic_err(format!(
            "insufficient funds to burn: balance={}, required={}",
            account_balance, amount_raw
        )));
    }
    account_balance -= amount_raw;

    let mut balances_store = PrefixedStorage::new(PREFIX_BALANCES, &mut deps.storage);
    balances_store.set(owner_address_raw.as_slice(), &account_balance.to_be_bytes())?;

    let mut config_store = PrefixedStorage::new(PREFIX_CONFIG, &mut deps.storage);
    let data = config_store
        .get(KEY_TOTAL_SUPPLY)
        .expect("could not read total supply")
        .expect("no total supply data stored");
    let mut total_supply = bytes_to_u128(&data).unwrap();

    total_supply -= amount_raw;

    config_store.set(KEY_TOTAL_SUPPLY, &total_supply.to_be_bytes())?;

    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "burn"),
            log(
                "account",
                deps.api.human_address(&env.message.sender)?.as_str(),
            ),
            log("amount", amount),
        ],
        data: None,
    };

    Ok(res)
}


/// Mint tokens
///
/// Add `amount` tokens to the recipient account, signer must be owner
///
/// @param amount the amount of money to mint
fn try_mint<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    state: State,
    recipient: &HumanAddr,
    amount: &str,
) -> HandleResult {
    let amount_raw = parse_u128(amount)?;
    let recipient_raw = deps.api.canonical_address(recipient)?;

    let mut account_balance = read_balance(&deps.storage, &recipient_raw)?;

    account_balance += amount_raw;

    let mut balances_store = PrefixedStorage::new(PREFIX_BALANCES, &mut deps.storage);

    balances_store.set(recipient_raw.as_slice(), &account_balance.to_be_bytes())?;

    let mut config_store = PrefixedStorage::new(PREFIX_CONFIG, &mut deps.storage);
    
    if env.message.sender != state.minter {
        panic!("not authorized minter")
        // Err(unauthorized());
        // todo ^^^ cannot infer type for type parameter `T` declared on the enum `Result`
    }

    let supply_data = config_store
        .get(KEY_TOTAL_SUPPLY)
        .expect("could not read total supply")
        .expect("no total supply data stored");
    let mut total_supply = bytes_to_u128(&supply_data).unwrap();

    total_supply += amount_raw;

    config_store.set(KEY_TOTAL_SUPPLY, &total_supply.to_be_bytes())?;

    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "mint"),
            log(
                "account", recipient.as_str(),
            ),
            log("amount", amount),
        ],
        data: None,
    };

    Ok(res)
}

fn perform_transfer<T: Storage>(
    store: &mut T,
    from: &CanonicalAddr,
    to: &CanonicalAddr,
    amount: u128,
) -> StdResult<()> {
    let mut balances_store = PrefixedStorage::new(PREFIX_BALANCES, store);

    let mut from_balance = read_u128(&balances_store, from.as_slice())?;
    if from_balance < amount {
        return Err(generic_err(format!(
            "Insufficient funds: balance={}, required={}",
            from_balance, amount
        )));
    }
    from_balance -= amount;
    balances_store.set(from.as_slice(), &from_balance.to_be_bytes())?;

    let mut to_balance = read_u128(&balances_store, to.as_slice())?;
    to_balance += amount;
    balances_store.set(to.as_slice(), &to_balance.to_be_bytes())?;

    Ok(())
}

// Converts 16 bytes value into u128
// Errors if data found that is not 16 bytes
pub fn bytes_to_u128(data: &[u8]) -> StdResult<u128> {
    match data[0..16].try_into() {
        Ok(bytes) => Ok(u128::from_be_bytes(bytes)),
        Err(_) => Err(generic_err("Corrupted data found. 16 byte expected.")),
    }
}

// Reads 16 byte storage value into u128
// Returns zero if key does not exist. Errors if data found that is not 16 bytes
pub fn read_u128<S: ReadonlyStorage>(store: &S, key: &[u8]) -> StdResult<u128> {
    let result = store.get(key)?;
    match result {
        Some(data) => bytes_to_u128(&data),
        None => Ok(0u128),
    }
}

// Source must be a decadic integer >= 0
pub fn parse_u128(source: &str) -> StdResult<u128> {
    match source.parse::<u128>() {
        Ok(value) => Ok(value),
        Err(_) => Err(generic_err("Error while parsing string to u128")),
    }
}

fn read_balance<S: Storage>(store: &S, owner: &CanonicalAddr) -> StdResult<u128> {
    let balance_store = ReadonlyPrefixedStorage::new(PREFIX_BALANCES, store);
    read_u128(&balance_store, owner.as_slice())
}

fn read_allowance<S: Storage>(
    store: &S,
    owner: &CanonicalAddr,
    spender: &CanonicalAddr,
) -> StdResult<u128> {
    let allowances_store = ReadonlyPrefixedStorage::new(PREFIX_ALLOWANCES, store);
    let owner_store = ReadonlyPrefixedStorage::new(owner.as_slice(), &allowances_store);
    read_u128(&owner_store, spender.as_slice())
}

fn write_allowance<S: Storage>(
    store: &mut S,
    owner: &CanonicalAddr,
    spender: &CanonicalAddr,
    amount: u128,
) -> StdResult<()> {
    let mut allowances_store = PrefixedStorage::new(PREFIX_ALLOWANCES, store);
    let mut owner_store = PrefixedStorage::new(owner.as_slice(), &mut allowances_store);
    owner_store.set(spender.as_slice(), &amount.to_be_bytes())?;
    Ok(())
}

fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() < 3 || bytes.len() > 30 {
        return false;
    }
    true
}

fn is_valid_symbol(symbol: &str) -> bool {
    let bytes = symbol.as_bytes();
    if bytes.len() < 3 || bytes.len() > 6 {
        return false;
    }
    for byte in bytes.iter() {
        if *byte < 65 || *byte > 90 {
            return false;
        }
    }
    true
}
