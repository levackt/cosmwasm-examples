use cosmwasm_std::testing::{mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{log, coin, to_binary, from_binary, from_slice, to_vec,
                   coins, Api, BankMsg,
                   Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern, HandleResponse,
                   HandleResult, InitResponse, InitResult, Querier, StdResult, Storage,
                   Uint128, ReadonlyStorage, HumanAddr, StdError};
use crate::coin_helpers::assert_sent_sufficient_coin;
use crate::msg::{HandleMsg, InitMsg, QueryMsg, PollResponse, TokenStakeResponse, CreatePollResponse};
use crate::state::{config, config_read, bank, bank_read, poll, poll_read,
                   State, TokenManager, Poll, PollStatus
};
use std::convert::TryInto;

use crate::contract::{handle, init, query};

const VOTING_TOKEN: &'static str = "secret";


#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::{coins, Api, HumanAddr, StdError};

    fn mock_init(mut deps: &mut Extern<MockStorage, MockApi, MockQuerier>) {
        let msg = InitMsg {
            denom: String::from(VOTING_TOKEN),
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
            denom: String::from(VOTING_TOKEN),
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
                denom: String::from(VOTING_TOKEN),
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
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "quorum_percentage must be 0 to 100"),
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
    }

    fn create_poll_msg(quorum_percentage: u8, description: String,
                       start_height: Option<u64>, end_height: Option<u64>) -> HandleMsg {
        let msg = HandleMsg::CreatePoll {
            quorum_percentage: Some(quorum_percentage),
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
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );

        //confirm poll count
        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                denom: String::from(VOTING_TOKEN),
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
    fn create_poll_no_quorum() {

        let mut deps = mock_dependencies(20, &[]);
        mock_init(&mut deps);
        let env = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(0, "test".to_string(),
                                  None, None);

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
                log("quorum_percentage", "0"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );

        //confirm poll count
        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                denom: String::from(VOTING_TOKEN),
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


        let handle_res = handle(&mut deps, env.clone(), msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
                log("quorum_percentage", "30"),
                log("end_height", "10001"),
                log("start_height", "0"),
            ]
        );

        let res = query(&deps, QueryMsg::Poll {
            poll_id: 1
        }).unwrap();
        let value: PollResponse = from_binary(&res).unwrap();
        assert_eq!(Some(10001), value.end_height);

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
    fn happy_days_end_poll_one_vote() {

        let mut deps = mock_dependencies(20, &coins(1000, VOTING_TOKEN));
        mock_init(&mut deps);
        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = create_poll_msg(0,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env.clone(), msg);
        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(1, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

        let env = mock_env(&deps.api, "voter", &coins(1, VOTING_TOKEN));
        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(1u128),
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
    fn end_poll_zero_quorum() {

        let mut deps = mock_dependencies(20, &coins(1000, VOTING_TOKEN));
        mock_init(&mut deps);
        let env = mock_env_height(&deps.api, "creator",
                                  &coins(2, VOTING_TOKEN),
                                  1000,
                                  10000);

        let msg = create_poll_msg(0,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, env.clone(), msg);

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, env.clone(), msg).unwrap();

        assert_eq!(
            handle_res.log,
            vec![
                log("action", "end_poll"),
                log("poll_id", "1"),
                log("rejected_reason", "No votes"),
                log("passed", "false"),
            ]
        );
    }


    #[test]
    fn end_poll_quorum_rejected() {
        let mut deps = mock_dependencies(20, &coins(100, VOTING_TOKEN));
        mock_init(&mut deps);
        let creatorEnv = mock_env(&deps.api, "creator", &coins(2, VOTING_TOKEN));

        let msg = create_poll_msg(30,"test".to_string(), None, None);

        let handle_res = handle(&mut deps, creatorEnv.clone(), msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(100, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

        let msg = HandleMsg::CastVote {
            poll_id: 1,
            encrypted_vote: "yes".to_string(),
            weight: Uint128::from(10u128),
        };
        let handle_res = handle(&mut deps, env.clone(), msg).unwrap();

        assert_eq!(
            handle_res.log,
            vec![
                log("action", "vote_casted"),
                log("poll_id", "1"),
                log("weight", "10"),
                log("voter", "voter1"),
            ]
        );

        let msg = HandleMsg::EndPoll {
            poll_id: 1
        };

        let handle_res = handle(&mut deps, creatorEnv.clone(), msg.clone()).unwrap();
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
                log("quorum_percentage", "10"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(100, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter2", &coins(1000, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

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

        let handle_res = handle(&mut deps, env.clone(), msg.clone()).unwrap();
        assert_eq!(
            handle_res.log,
            vec![
                log("action", "create_poll"),
                log("creator", "creator"),
                log("poll_id", "1"),
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "10001"),
            ]
        );

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
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "0"),
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
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter1", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env, msg.clone()).unwrap();

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

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                denom: String::from(VOTING_TOKEN),
                owner: deps
                    .api
                    .canonical_address(&HumanAddr::from("creator"))
                    .unwrap(),
                poll_count: 0,
                staked_tokens: Uint128::from(11u128),
            }
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
                denom: String::from(VOTING_TOKEN),
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
                log("quorum_percentage", "30"),
                log("end_height", "0"),
                log("start_height", "0"),
            ]
        );
        //end todo 1. extract create_poll

        let msg = HandleMsg::StakeVotingTokens {  };
        let env = mock_env(&deps.api, "voter", &coins(11, VOTING_TOKEN));

        let handle_res = handle(&mut deps, env.clone(), msg.clone()).unwrap();

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

        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                denom: String::from(VOTING_TOKEN),
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
