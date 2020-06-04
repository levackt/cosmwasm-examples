use cosmwasm_std::{CanonicalAddr, HumanAddr, Env, Storage, Uint128, StdResult};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton, ReadonlyPrefixedStorage, sequence, nextval, currval,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

static CONFIG_KEY: &[u8] = b"config";
static POLL_ID: &[u8] = b"poll_id";
static POLL_KEY: &[u8] = b"polls";
static POLL_VOTERS_KEY: &[u8] = b"poll_voters";
static POLL_VOTER_INFO_KEY: &[u8] = b"poll_voter_info";
static LOCKED_TOKENS_KEY: &[u8] = b"locked_tokens";
static BANK_KEY: &[u8] = b"bank";
pub const PREFIX_VOTERS: &[u8] = b"voters";
pub const PREFIX_ALLOWANCES: &[u8] = b"allowances";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub denom: String,
    pub owner: CanonicalAddr,
    pub poll_count: u64,
    pub staked_tokens: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct TokenManager {
    pub token_balance: Uint128,
    // pub locked_tokens: HashMap<u64, Uint128>,
    //todo map poll_id to weight voted, mainly to find the largest weight staked at withdrawal time
    pub participated_polls: Vec<u64>, // todo set of polls for the voter
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

pub fn poll_voters<S: Storage>(storage: &mut S) -> Bucket<S, Voter> {
    bucket(POLL_VOTERS_KEY, storage)
}

pub fn poll_voters_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Voter> {
    bucket_read( POLL_VOTERS_KEY, storage)
}

pub fn poll_voter_info<S: Storage>(storage: &mut S) -> Bucket<S, Voter> {
    bucket(POLL_VOTER_INFO_KEY, storage)
}

pub fn poll_voter_info_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Voter> {
    bucket_read( POLL_VOTER_INFO_KEY, storage)
}

pub fn locked_tokens<S: Storage>(storage: &mut S) -> Bucket<S, Uint128> {
    bucket(LOCKED_TOKENS_KEY, storage)
}

pub fn locked_tokens_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Uint128> {
    bucket_read( LOCKED_TOKENS_KEY, storage)
}

pub fn next_poll_id<S: Storage>(storage: &mut S) -> StdResult<u64> {
    let mut seq = sequence(storage, POLL_ID);
    nextval(&mut seq)
}

//todo currval for poll count if needed
// pub fn curr_poll_id<S: Storage>(storage: &S) -> StdResult<u64> {
//     let mut seq = singleton(storage, POLL_ID);
//     currval(&mut seq)
// }
