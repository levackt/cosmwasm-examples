use cosmwasm_std::{generic_err, log, coin, to_binary, from_binary, to_vec,
                   Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern, HandleResponse,
                   HandleResult, InitResponse, InitResult, Querier, StdResult, Storage,
                   Uint128, ReadonlyStorage, HumanAddr};
use cosmwasm_std::testing::{mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_storage::ReadonlyPrefixedStorage;
use crate::coin_helpers::assert_sent_sufficient_coin;
use crate::msg::{HandleMsg, InitMsg, QueryMsg, PollResponse, TokenStakeResponse, CreatePollResponse,
                 PollCountResponse};
use crate::state::{config, config_read, bank, bank_read, poll, poll_read, poll_voters,
                   poll_voter_info, poll_voter_info_read, next_poll_id,
                   locked_tokens, locked_tokens_read,
                   State, TokenManager, Poll, PollStatus, Voter};
use std::convert::TryInto;
use std::collections::{HashMap, HashSet};


const MIN_STAKE_AMOUNT: u128 = 10;
const MIN_DESC_LENGTH: usize = 3;
const MAX_DESC_LENGTH: usize = 64;
const VOTING_TOKEN: &'static str = "voting_token";

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> InitResult {
    let state = State {
        token: deps.api.canonical_address(&msg.token)?,
        owner: env.message.sender.clone(),
        poll_count: 0,
        staked_tokens: Uint128::zero(),
    };

    config(&mut deps.storage).save(&state)?;

    Ok(InitResponse::default())
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

// Converts 16 bytes value into u128
// Errors if data found that is not 16 bytes
pub fn bytes_to_u128(data: &[u8]) -> StdResult<u128> {
    match data[0..16].try_into() {
        Ok(bytes) => Ok(u128::from_be_bytes(bytes)),
        Err(_) => Err(generic_err("Corrupted data found. 16 byte expected.")),
    }
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {

    match msg {
        HandleMsg::StakeVotingTokens { } => stake_voting_tokens(deps, env),
        HandleMsg::WithdrawVotingTokens { amount} => withdraw_voting_tokens(deps, env, amount),
        HandleMsg::CastVote {
            poll_id,
            encrypted_vote,
            weight
        } => cast_vote(deps, env, poll_id, encrypted_vote, weight),
        HandleMsg::EndPoll {
            poll_id,
        } => end_poll(deps, env, poll_id),
        HandleMsg::CreatePoll {
            quorum_percentage,
            description,
            start_height,
            end_height
        } => create_poll(deps, env, quorum_percentage, description, start_height, end_height),
    }
}

pub fn stake_voting_tokens<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> HandleResult {
    assert_sent_sufficient_coin(&env.message.sent_funds,
                                Some(coin(MIN_STAKE_AMOUNT, VOTING_TOKEN)))?;
    let key = &env.message.sender.as_slice();

    let balance = Uint128::zero();

    let mut token_manager = match bank_read(&deps.storage).may_load(key)? {
        Some(token_manager) => Some(token_manager),
        None => Some(TokenManager {
            token_balance: balance,
            participated_polls: Vec::new()
        }),
    }.unwrap();

    let sent_funds = env.message.sent_funds.iter().find(|coin| {
        coin.denom == VOTING_TOKEN
    }).unwrap();

    token_manager.token_balance = token_manager.token_balance + sent_funds.amount;

    let mut state = config(&mut deps.storage).load()?;
    let staked_tokens = state.staked_tokens.u128() + sent_funds.amount.u128();
    state.staked_tokens = Uint128::from(staked_tokens);
    config(&mut deps.storage).save(&state)?;

    bank(&mut deps.storage).save(key, &token_manager)?;

    //todo confirm only VOTING_TOKEN
    send_tokens(
        &deps.api,
        &env.message.sender,
        &env.contract.address,
        env.message.sent_funds,
        "approve",
    )
}

// Withdraw amount if not staked. By default all funds will be withdrawn.
pub fn withdraw_voting_tokens<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<Uint128>
) -> HandleResult {

    let key = &env.message.sender.as_slice();

    if let Some(mut token_manager) = bank_read(&deps.storage).may_load(key)? {
        let largest_staked = locked_amount(&env.message.sender, deps);
        let withdraw_amount = match amount {
            Some(amount) => Some(amount.u128()),
            None => Some(token_manager.token_balance.u128()),
        }.unwrap();
        if largest_staked + withdraw_amount > token_manager.token_balance.u128()  {
            Err(generic_err("User is trying to withdraw too many tokens."))
        } else {

            let balance = token_manager.token_balance.u128() - withdraw_amount;
            token_manager.token_balance = Uint128::from(balance);

            bank(&mut deps.storage).save(key, &token_manager)?;

            let mut state = config(&mut deps.storage).load()?;
            let staked_tokens = state.staked_tokens.u128() - withdraw_amount;
            state.staked_tokens = Uint128::from(staked_tokens);
            config(&mut deps.storage).save(&state)?;

            send_tokens(
                &deps.api,
                &env.contract.address,
                &env.message.sender,
                vec![coin(withdraw_amount, VOTING_TOKEN)],
                "approve",
            )
        }
    } else {
        Err(generic_err("Nothing staked"))
    }
}

fn invalid_char(c: char) -> bool {
    let is_valid =
        (c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c == '.' || c == '-' || c == '_' || c == ' ');
    !is_valid
}

/// validate_description returns an error if the description is invalid
/// (we require 3-64 lowercase ascii letters, numbers, or . - _)
fn validate_description(description: &str) -> StdResult<()> {
    if description.len() < MIN_DESC_LENGTH {
        Err(generic_err("Description too short"))
    } else if description.len() > MAX_DESC_LENGTH {
        Err(generic_err("Description too long"))
    } else {
        match description.find(invalid_char) {
            None => Ok(()),
            Some(bytepos_invalid_char_start) => {
                let c = description[bytepos_invalid_char_start..].chars().next().unwrap();
                Err(generic_err(format!("Invalid character: '{}'", c)))
            }
        }
    }
}

/// validate_quorum_percentage returns an error if the quorum_percentage is invalid
/// (we require 1-100)
fn validate_quorum_percentage(quorum_percentage: u8) -> StdResult<()> {
    if quorum_percentage <= 0 || quorum_percentage > 100 {
        Err(generic_err("quorum_percentage must be 1 to 100"))
    } else {
        Ok(())
    }
}

/// create a new poll
pub fn create_poll<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    // state: State,
    quorum_percentage: u8,
    description: String,
    start_height: Option<u64>,
    end_height: Option<u64>,
) -> StdResult<HandleResponse> {

    validate_quorum_percentage(quorum_percentage)?;
    validate_description(&description)?;

    let mut state = config(&mut deps.storage).load()?;
    let poll_count = state.poll_count;
    let poll_id = poll_count + 1;
    state.poll_count = poll_id;

    let new_poll = Poll {
        creator: env.message.sender,
        status : PollStatus::InProgress,
        quorum_percentage,
        yes_votes: Uint128::zero(),
        no_votes: Uint128::zero(),
        voters: vec![],
        voter_info: vec![],
        end_height,
        start_height,
        description,
    };
    let key = state.poll_count.to_string();
    poll(&mut deps.storage).save(key.as_bytes(), &new_poll)?;

    config(&mut deps.storage).save(&state)?;

    let r = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "create_poll"),
            log(
                "creator",
                deps.api.human_address(&new_poll.creator)?.as_str(),
            ),
            log("poll_id", &poll_id.to_string()),
        ],
        data: None,
    };
    Ok(r)
}

/*
 * Ends a poll. Only the creator of a given poll can end that poll.
 */
pub fn end_poll<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    poll_id: u64,
) -> HandleResult {

    let key = &poll_id.to_string();
    if (poll(&mut deps.storage).may_load(key.as_bytes())?).is_none() {
        return Err(generic_err("Poll does not exist"));
    }

    let mut a_poll = poll(&mut deps.storage).load(key.as_bytes()).unwrap();

    if a_poll.creator != env.message.sender {
        return Err(generic_err("User is not the creator of the poll."));
    }

    if a_poll.status != PollStatus::InProgress {
        return Err(generic_err("Poll is not in progress"));
    }

    if a_poll.start_height.is_some() && a_poll.start_height.unwrap() > env.block.height {
        return Err(generic_err("Voting period has not started."));
    }

    if a_poll.end_height.is_some() && a_poll.end_height.unwrap() > env.block.height {
        return Err(generic_err("Voting period has not expired."));
    }

    let mut no = 0u128;
    let mut yes = 0u128;

    for voter in &a_poll.voter_info {
        // todo abstain and veto
        if voter.vote == "yes" {
            yes += voter.weight.u128();
        } else {
            no += voter.weight.u128();
        }
    }
    let tallied_weight = yes + no;

    let poll_status = PollStatus::Rejected;
    let mut rejected_reason = "";
    let mut passed = false;

    if tallied_weight > 0 {
        let contract_address_human = deps.api.human_address(&env.contract.address)?;

        let staked_weight = deps.querier.query_balance(
            contract_address_human,VOTING_TOKEN).unwrap().amount;

        let quorum = ((tallied_weight / staked_weight.u128()) * 100) as u8;

        if quorum < a_poll.quorum_percentage {
            // Quorum: More than quorum_percentage of the total staked tokens at the end of the voting
            // period need to have participated in the vote.
            rejected_reason = "Quorum not reached";
        } else if yes > tallied_weight / 2 {
            //Threshold: More than 50% of the tokens that participated in the vote
            // (after excluding “Abstain” votes) need to have voted in favor of the proposal (“Yes”).
            a_poll.status = PollStatus::Passed;
            passed = true;
        } else {
            rejected_reason = "Threshold not reached";
        }
    } else {
        rejected_reason = "Quorum not reached";
    }
    a_poll.status = poll_status;
    poll(&mut deps.storage).save(key.as_bytes(), &a_poll)?;


    for voter in &a_poll.voters {
        locked_tokens(&mut deps.storage).save(get_poll_voter_key(poll_id, &voter).as_bytes(),
                                                &Uint128::zero());
    }

    let mut log = vec![
        log("action", "end_poll"),
        log("poll_id", &poll_id.to_string()),
        log("rejected_reason", rejected_reason),
        log("passed", &passed.to_string()),
    ];

    let r = HandleResponse {
        messages: vec![],
        log,
        data: None,
    };
    Ok(r)
}

// finds the largest locked amount in participated polls.
fn locked_amount<S: Storage, A: Api, Q: Querier>(voter: &CanonicalAddr, deps: &mut Extern<S, A, Q>) -> u128 {

    let mut largest = 0u128;

    let voter_key = &voter.as_slice();
    let token_manager = bank_read(&deps.storage).load(voter_key).unwrap();

    token_manager.participated_polls.iter().for_each(| poll_id | {
        let poll_key = get_poll_voter_key(*poll_id, &voter);
        let voter_info = poll_voter_info_read(&deps.storage).load(poll_key.as_bytes()).unwrap();

        if voter_info.weight.u128() > largest {
            largest = voter_info.weight.u128();
        }
    });

    largest
}

fn has_voted(voter: &CanonicalAddr, a_poll: &Poll) -> bool {
    return a_poll.voters.contains(voter)
}

pub fn cast_vote<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    poll_id: u64,
    vote: String,
    weight: Uint128,
) -> HandleResult {

    let poll_key = &poll_id.to_string();
    if (poll(&mut deps.storage).may_load(poll_key.as_bytes())?).is_none() {
        return Err(generic_err("Poll does not exist"));
    }

    let mut a_poll = poll(&mut deps.storage).load(poll_key.as_bytes()).unwrap();

    if a_poll.status != PollStatus::InProgress {
        return Err(generic_err("Poll is not in progress"));
    }

    if has_voted(&env.message.sender, &a_poll) {
        return Err(generic_err("User has already voted."));
    }

    let voter_key = &env.message.sender.as_slice();
    let mut token_manager = match bank_read(&deps.storage).may_load(voter_key)? {
        Some(token_manager) => Some(token_manager),
        None => Some(TokenManager {
            token_balance: Uint128::zero(),
            participated_polls: Vec::new(),
        }),
    }.unwrap();

    if &token_manager.token_balance < &weight {
        return Err(generic_err("User does not have enough staked tokens."));
    }
    token_manager.participated_polls.push(poll_id);
    a_poll.voters.push(env.message.sender.clone());

    let voter_info = Voter {
        vote,
        weight
    };

    poll_voter_info(&mut deps.storage).save(
        get_poll_voter_key(poll_id, &env.message.sender).as_bytes(),
        &voter_info)?;

    a_poll.voter_info.push(voter_info);
    poll(&mut deps.storage).save(poll_key.as_bytes(), &a_poll)?;
    bank(&mut deps.storage).save(voter_key, &token_manager)?;

    let log = vec![
        log("action", "vote_casted"),
        log("poll_id", &poll_id.to_string()),
        log("weight", &weight.to_string()),
        log("voter", deps.api.human_address((&env.message.sender)).unwrap().as_str()),
    ];

    let r = HandleResponse {
        messages: vec![],
        log,
        data: None,
    };
    Ok(r)
}

// todo maybe redundant once storage is optimized
fn get_poll_voter_key(poll_id: u64, voter_address: &CanonicalAddr) -> String {
    let poll_voter_key = poll_id.to_string() + &voter_address.to_string();
    poll_voter_key
}

fn send_tokens<A: Api>(
    api: &A,
    from_address: &CanonicalAddr,
    to_address: &CanonicalAddr,
    amount: Vec<Coin>,
    action: &str,
) -> HandleResult {
    let from_human = api.human_address(from_address)?;
    let to_human = api.human_address(to_address)?;
    let log = vec![log("action", action), log("to", to_human.as_str())];

    let r = HandleResponse {
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: from_human,
            to_address: to_human,
            amount,
        })],
        log,
        data: None,
    };
    Ok(r)
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    _deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&config_read(&_deps.storage).load()?),

        QueryMsg::TokenStake { address } => {
            token_balance(_deps, address)
        }
        QueryMsg::Poll { poll_id } => {
            query_poll(_deps, poll_id)
        }
    }
}

fn query_poll<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    poll_id: u64,
) -> StdResult<Binary> {

    let key = &poll_id.to_string();

    let poll = match poll_read(&deps.storage).may_load(key.as_bytes())? {
        Some(poll) => Some(poll),
        None => return Err(generic_err("Poll does not exist")),
    }.unwrap();

    let resp = PollResponse {
        creator: deps.api.human_address(&poll.creator).unwrap(),
        status: poll.status,
        quorum_percentage: poll.quorum_percentage,
        end_height: poll.end_height,
        start_height: poll.start_height,
        description: poll.description,
    };
    to_binary(&resp)

}

fn token_balance<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<Binary> {

    let key = deps.api.canonical_address(&address).unwrap();

    let token_manager = match bank_read(&deps.storage).may_load(key.as_slice())? {
        Some(token_manager) => Some(token_manager),
        None => Some(TokenManager {
            token_balance: Uint128::zero(),
            participated_polls: Vec::new()
        }),
    }.unwrap();

    let resp = TokenStakeResponse { token_balance: token_manager.token_balance };

    to_binary(&resp)
}


#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::{coins, Api, HumanAddr, StdError};

    fn mock_init(mut deps: &mut Extern<MockStorage, MockApi, MockQuerier>) {
        let msg = InitMsg {
            token: HumanAddr::from(VOTING_TOKEN),
        };

        let env = mock_env(&deps.api, "creator", &coins(2, "token"));
        let _res = init(&mut deps, env, msg).expect("contract successfully handles InitMsg");
    }

    fn mock_env_height<A: Api>(
        api: &A,
        sender: &str,
        sent: &[Coin],
        height: u64,
        time: u64,
    ) -> Env {
        let mut env = mock_env(api, sender, sent);
        env.block.height = height;
        env.block.time = time;
        env
    }

    fn init_msg() -> InitMsg {
        InitMsg {
            token: HumanAddr::from(VOTING_TOKEN),
        }
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = init_msg();
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                token: deps
                    .api
                    .canonical_address(&HumanAddr::from(VOTING_TOKEN))
                    .unwrap(),

                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 0,
                staked_tokens: Uint128::zero(),
            }
        );
    }

    #[test]
    fn poll_not_found() {
        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let res = query(&deps, QueryMsg::Poll {
            poll_id: 1
        });

        match res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Poll does not exist"),
            Err(e) => panic!("Unexpected error: {:?}", e),
            _ => panic!("Must return error"),
        }
    }


    #[test]
    fn fails_create_poll_invalid_quorum_percentage() {

        let mut deps = mock_dependencies(20, &[]);
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let msg = create_poll_msg(101,"test".to_string(), None, None);

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "quorum_percentage must be 1 to 100"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_create_poll_invalid_description() {

        let mut deps = mock_dependencies(20, &[]);
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let msg = create_poll_msg(30, "a".to_string(),
                                  None, None);

        match handle(&mut deps, env.clone(), msg) {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Description too short"),
            Err(_) => panic!("Unknown error"),
        }

        let msg = create_poll_msg(
            100,
            "01234567890123456789012345678901234567890123456789012345678901234".to_string(),
            None, None);


        match handle(&mut deps, env.clone(), msg) {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Description too long"),
            Err(_) => panic!("Unknown error"),
        }

        let msg = create_poll_msg(100,"Loud".to_string(), None, None);

        match handle(&mut deps, env.clone(), msg) {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg.as_str(), "Invalid character: 'L'"),
            Err(_) => panic!("Unknown error"),
        }
    }

    fn create_poll_msg(quorum_percentage: u8, description: String,
                       start_height: Option<u64>, end_height: Option<u64>) -> HandleMsg {
        let msg = HandleMsg::CreatePoll {
            quorum_percentage,
            description,
            start_height,
            end_height,
        };
        msg
    }

    #[test]
    fn create_poll() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30, "test".to_string(),
                                  None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );

        //confirm poll count
        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                token: deps
                    .api
                    .canonical_address(&HumanAddr::from(VOTING_TOKEN))
                    .unwrap(),

                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 1,
                staked_tokens: Uint128::zero(),
            }
        );
    }

    #[test]
    fn fails_end_poll_before_end_height() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = create_poll_msg(30,"test".to_string(), None, Some(10001));

        let handle_res = handle(&mut deps, env.clone(), msg);

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, env.clone(), msg);

        match handle_res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Voting period has not expired."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn happy_days_end_poll() {

        let mut deps = mock_dependencies(20, &coins(1000, VOTING_TOKEN));
        mock_init(&mut deps);
        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env.clone(), msg);

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(1000, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

        let env = mock_env(&deps.api, "voter", &coins(1000, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1000u128),
        };
        let res = handle(&mut deps, env.clone(), msg);

        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, env.clone(), msg).unwrap();

        assert_eq!(
            handle_res.log,
            vec![
                log("action", "end_poll"),
                log("poll_id", "1"),
                log("rejected_reason", ""),
                log("passed", "true"),
            ]
        );
    }

    #[test]
    fn end_poll_quorum_rejected() {
        let mut deps = mock_dependencies(20, &coins(1000, VOTING_TOKEN));
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(11, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract stake #2
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter2", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter2"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(11, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract cast_vote
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
        };
        let res = handle(&mut deps, env, msg);

        let env = mock_env(&deps.api, "creator", &coins(11, VOTING_TOKEN));

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "end_poll"),
                log("poll_id", "1"),
                log("rejected_reason", "Quorum not reached"),
                log("passed", "false"),
            ]
        );
    }

    #[test]
    fn end_poll_nay_rejected() {
        let mut deps = mock_dependencies(20, &coins(1000, VOTING_TOKEN));
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(10,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(100, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(100, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract stake #2
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter2", &coins(1000, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter2"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(1000, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract cast_vote
        let env = mock_env(&deps.api, "voter2", &[]);
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "no".to_string(),
            weight: Uint128::from(1000u128),
        };
        let res = handle(&mut deps, env, msg);

        let env = mock_env(&deps.api, "creator", &coins(1000, VOTING_TOKEN));

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };
        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "end_poll"),
                log("poll_id", "1"),
                log("rejected_reason", "Threshold not reached"),
                log("passed", "false"),
            ]
        );
    }

    #[test]
    fn fails_end_poll_before_start_height() {
        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = create_poll_msg(30,"test".to_string(), Some(10001), None);

        let handle_res = handle(&mut deps, env.clone(), msg);

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, env.clone(), msg);

        match handle_res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Voting period has not started."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_cast_vote_not_enough_staked() {
        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );
        //end todo 1. extract create_poll

        // todo extract cast_vote
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
        };

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "User does not have enough staked tokens."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn happy_days_cast_vote() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );

        // todo extract stake
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter1"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(11, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract cast_vote
        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(10u128),
        };

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

        assert_eq!(
            handle_res.log,
            vec![
                log("action", "vote_casted"),
                log("poll_id", "1"),
                log("weight", "10"),
                log("voter", "voter1"),
            ]
        );
    }

    #[test]
    fn happy_days_withdraw_voting_tokens() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                token: deps
                    .api
                    .canonical_address(&HumanAddr::from(VOTING_TOKEN))
                    .unwrap(),

                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 0,
                staked_tokens: Uint128::from(11u128),
            }
        );

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter1"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(11, VOTING_TOKEN),
            })
        );

        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::WithdrawVotingTokens {
            amount: Some(Uint128::from(11u128)),
        };

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("cosmos2contract"),
                to_address: HumanAddr::from("voter1"),
                amount: coins(11, VOTING_TOKEN),
            })
        );

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                token: deps
                    .api
                    .canonical_address(&HumanAddr::from(VOTING_TOKEN))
                    .unwrap(),

                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 0,
                staked_tokens: Uint128::zero(),
            }
        );
    }

    #[test]
    fn fails_withdraw_voting_tokens_no_stake() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);

        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::WithdrawVotingTokens {
            amount: Some(Uint128::from(11u128)),
        };

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Nothing staked"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_withdraw_too_many_tokens() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(10, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

        let env = mock_env(&deps.api, "voter1", &[]);
        let msg = HandleMsg::WithdrawVotingTokens {
            amount: Some(Uint128::from(11u128)),
        };

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "User is trying to withdraw too many tokens."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_cast_vote_twice() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
            ]
        );
        //end todo 1. extract create_poll

        // todo extract stake
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        assert_eq!(
            msg,
            &CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from("voter"),
                to_address: HumanAddr::from("cosmos2contract"),
                amount: coins(11, VOTING_TOKEN),
            })
        );
        // end extract stake

        // todo extract cast_vote
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
        };
        let res = handle(&mut deps, env.clone(), msg);

        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
        };
        let res = handle(&mut deps, env.clone(), msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "User has already voted."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_cast_vote_without_poll() {
        let mut deps = mock_dependencies(20, &[]);
        let msg = HandleMsg::CastVote {
            poll_id: 0,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
        };
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Poll does not exist"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }


    #[test]
    fn happy_days_stake_voting_tokens() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(1, handle_res.messages.len());
        let msg = handle_res.messages.get(0).expect("no message");

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                token: deps
                    .api
                    .canonical_address(&HumanAddr::from(VOTING_TOKEN))
                    .unwrap(),

                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 0,
                staked_tokens: Uint128::from(11u128),
            }
        );
    }

    #[test]
    fn fails_insufficient_funds() {
        let mut deps = mock_dependencies(20, &[]);

        // initialize the store
        let msg = init_msg();
        let env = mock_env(&deps.api, "voter", &coins(2, VOTING_TOKEN));
        let init_res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, init_res.messages.len());

        // insufficient token
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(0, VOTING_TOKEN));

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Insufficient funds sent"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn fails_staking_wrong_token() {
        let mut deps = mock_dependencies(20, &[]);

        // initialize the store
        let msg = init_msg();
        let env = mock_env(&deps.api, "voter", &coins(2, VOTING_TOKEN));
        let init_res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, init_res.messages.len());


        // wrong token
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(11, "play money"));

        let res = handle(&mut deps, env, msg);

        match res {
            Ok(_) => panic!("Must return error"),
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Insufficient funds sent"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

}
