use cosmwasm_std::testing::{mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{generic_err, log, unauthorized, coin, to_binary, from_binary, from_slice, to_vec,
                   coins, Api, BankMsg,
                   Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern, HandleResponse,
                   HandleResult, InitResponse, InitResult, Querier, StdResult, Storage,
                   Uint128, ReadonlyStorage, HumanAddr};
use crate::coin_helpers::assert_sent_sufficient_coin;
use crate::msg::{HandleMsg, InitMsg, QueryMsg, PollResponse, TokenStakeResponse, CreatePollResponse};
use crate::state::{config, config_read, bank, bank_read, poll, poll_read,
                   State, TokenManager, Poll, PollStatus
};
use std::convert::TryInto;
use std::collections::{HashMap, HashSet};

use crate::contract::{handle, init, query};
//todo moving tests
