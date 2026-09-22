use crate::error::ContractError;
use crate::execute::admin::cleanup_orphan_markers::execute_cleanup_orphan_markers;
use crate::execute::admin::update_configuration::execute_update_configuration;
use crate::execute::contribution::cancel_contribution::execute_cancel_contribution;
use crate::execute::contribution::confirm_contribution::execute_confirm_contribution;
use crate::execute::contribution::initiate_contribution::execute_initiate_contribution;
use crate::execute::contribution::price_contribution::execute_price_contribution;
use crate::execute::contribution::submit_contribution::execute_submit_contribution;
use crate::execute::redemption::cancel_redemption::execute_cancel_redemption;
use crate::execute::redemption::complete_redemption_pool::execute_complete_redemption_pool;
use crate::execute::redemption::confirm_redemption::execute_confirm_redemption;
use crate::execute::redemption::initiate_redemption::execute_initiate_redemption;
use crate::execute::redemption::pool_redemption::execute_pool_redemption;
use crate::execute::swap::cancel_swap::execute_cancel_swap;
use crate::execute::swap::complete_swap_pool::execute_complete_swap_pool;
use crate::execute::swap::confirm_swap::execute_confirm_swap;
use crate::execute::swap::initiate_swap::execute_initiate_swap;
use crate::execute::swap::pool_swap::execute_pool_swap;
use crate::execute::swap::submit_swap::execute_submit_swap;
use crate::instantiate::instantiate_contract::instantiate_contract;
use crate::migrate::migrate_contract;
use crate::msg::{ExecuteMsg, InstantiateContractMsg, MigrateMsg, QueryMsg};
use crate::query::query_config::query_configuration;
use crate::query::query_contribution::query_pending_contribution;
use crate::query::query_contribution::query_pending_contribution_by_reference;
use crate::query::query_redemption::query_pending_redemption;
use crate::query::query_redemption::query_pending_redemption_by_reference;
use crate::query::query_swap::query_pending_swap;
use crate::query::query_swap::query_pending_swap_by_reference;
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response,
};

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateContractMsg,
) -> Result<Response, ContractError> {
    instantiate_contract(deps, env, msg)
}

#[entry_point]
pub fn migrate(deps: DepsMut, env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    migrate_contract(deps, env, msg)
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::InitiateContribution {
            contributor_address,
            contribution_ref,
        } => execute_initiate_contribution(deps, info, contributor_address, contribution_ref),
        ExecuteMsg::SubmitContribution {
            contribution_id,
            marker_denom,
        } => execute_submit_contribution(deps, env, info, contribution_id, marker_denom),
        ExecuteMsg::PriceContribution {
            contribution_id,
            mint_ldt_amount,
        } => execute_price_contribution(deps, info, contribution_id, mint_ldt_amount),
        ExecuteMsg::ConfirmContribution {
            contribution_id,
            expected_mint_ldt_amount,
        } => {
            execute_confirm_contribution(deps, env, info, contribution_id, expected_mint_ldt_amount)
        }
        ExecuteMsg::CancelContribution { contribution_id } => {
            execute_cancel_contribution(deps, env, info, contribution_id)
        }
        ExecuteMsg::InitiateRedemption {
            pool_denom,
            num_coins,
            redeemer_address,
            redemption_ref,
        } => execute_initiate_redemption(
            deps,
            info,
            env,
            pool_denom,
            num_coins,
            redeemer_address,
            redemption_ref,
        ),
        ExecuteMsg::PoolRedemption {
            redemption_id,
            scope_uuids,
        } => execute_pool_redemption(deps, info, env, redemption_id, scope_uuids),
        ExecuteMsg::CompleteRedemptionPool {
            redemption_id,
            burn_ldt_amount,
        } => execute_complete_redemption_pool(deps, info, redemption_id, burn_ldt_amount),
        ExecuteMsg::ConfirmRedemption { redemption_id } => {
            execute_confirm_redemption(deps, env, info, redemption_id)
        }
        ExecuteMsg::CancelRedemption { redemption_id } => {
            execute_cancel_redemption(deps, info, redemption_id)
        }
        ExecuteMsg::CleanupOrphanMarkers { marker_denoms } => {
            execute_cleanup_orphan_markers(deps, env, info, marker_denoms)
        }
        ExecuteMsg::UpdateConfiguration {
            otc_address,
            redemption_marker_admin,
        } => execute_update_configuration(deps, env, info, otc_address, redemption_marker_admin),
        ExecuteMsg::InitiateSwap {
            pool_denom,
            num_coins,
            contributor_address,
            swap_ref,
        } => execute_initiate_swap(
            deps,
            info,
            env,
            pool_denom,
            num_coins,
            contributor_address,
            swap_ref,
        ),
        ExecuteMsg::PoolSwap {
            swap_id,
            removed_scope_uuids,
        } => execute_pool_swap(deps, info, env, swap_id, removed_scope_uuids),
        ExecuteMsg::CompleteSwapPool {
            swap_id,
            mint_ldt_amount,
            burn_ldt_amount,
        } => execute_complete_swap_pool(deps, info, swap_id, mint_ldt_amount, burn_ldt_amount),
        ExecuteMsg::SubmitSwap {
            swap_id,
            incoming_marker_denom,
        } => execute_submit_swap(deps, env, info, swap_id, incoming_marker_denom),
        ExecuteMsg::ConfirmSwap { swap_id } => execute_confirm_swap(deps, env, info, swap_id),
        ExecuteMsg::CancelSwap { swap_id } => execute_cancel_swap(deps, env, info, swap_id),
    }
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::GetPendingContribution { contribution_id } => Ok(to_json_binary(
            &query_pending_contribution(deps, contribution_id)?,
        )?),
        QueryMsg::GetPendingRedemption { redemption_id } => Ok(to_json_binary(
            &query_pending_redemption(deps, redemption_id)?,
        )?),
        QueryMsg::GetPendingContributionByReference { contribution_ref } => Ok(to_json_binary(
            &query_pending_contribution_by_reference(deps, contribution_ref)?,
        )?),
        QueryMsg::GetPendingRedemptionByReference { redemption_ref } => Ok(to_json_binary(
            &query_pending_redemption_by_reference(deps, redemption_ref)?,
        )?),
        QueryMsg::GetPendingSwap { swap_id } => {
            Ok(to_json_binary(&query_pending_swap(deps, swap_id)?)?)
        }
        QueryMsg::GetPendingSwapByReference { swap_ref } => Ok(to_json_binary(
            &query_pending_swap_by_reference(deps, swap_ref)?,
        )?),
        QueryMsg::GetConfiguration {} => Ok(to_json_binary(&query_configuration(deps)?)?),
    }
}
