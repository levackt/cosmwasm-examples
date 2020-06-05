use cosmwasm_std::{CanonicalAddr, HumanAddr, Env, Storage, Uint128, StdResult};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton, ReadonlyPrefixedStorage
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

static CONFIG_KEY: &[u8] = b"config";
static POLL_KEY: &[u8] = b"polls";
static BANK_KEY: &[u8] = b"bank";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub denom: String,
    pub owner: CanonicalAddr,
    pub poll_count: u64,
    pub staked_tokens: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct TokenManager {
    pub token_balance: Uint128, // total staked balance
    pub locked_tokens: HashMap<u64, Uint128>, //maps poll_id to weight voted
    pub participated_polls: Vec<u64>, // poll_id
}

impl TokenManager {
    pub fn new() -> Self {
        let token_balance = Uint128::zero();
        let locked_tokens = HashMap::new();
        let participated_polls = Vec::new();
        TokenManager {
            token_balance,
            locked_tokens,
            participated_polls,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Voter {
    pub vote: String,
    pub weight: Uint128
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum PollStatus {
    InProgress,
    Tally,
    Passed,
    Rejected,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Poll {
    pub creator: CanonicalAddr,
    pub status : PollStatus,
    pub quorum_percentage: u8,
    pub yes_votes: Uint128,
    pub no_votes: Uint128,
    pub voters: Vec<CanonicalAddr>,
    pub voter_info: Vec<Voter>,
    pub end_height: Option<u64>,
    pub start_height: Option<u64>,
    pub description: String,
}

impl State {
}

pub fn config<S: Storage>(storage: &mut S) -> Singleton<S, State> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, State> {
    singleton_read(storage, CONFIG_KEY)
}

pub fn poll<S: Storage>(storage: &mut S) -> Bucket<S, Poll> {
    bucket(POLL_KEY, storage)
}

pub fn poll_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Poll> {
    bucket_read(POLL_KEY, storage)
}

pub fn bank<S: Storage>(storage: &mut S) -> Bucket<S, TokenManager> {
    bucket(BANK_KEY, storage)
}

pub fn bank_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, TokenManager> {
    bucket_read( BANK_KEY, storage)
}
