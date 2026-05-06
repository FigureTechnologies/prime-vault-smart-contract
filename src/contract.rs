use crate::error::ContractError;
use crate::execute::admin::cleanup_orphan_markers::execute_cleanup_orphan_markers;
use crate::execute::contribution::cancel_contribution::execute_cancel_contribution;
use crate::execute::contribution::finalize_contribution::execute_finalize_contribution;
use crate::execute::contribution::initiate_contribution::execute_initialize_contribution;
use crate::execute::redemption::cancel_redemption::execute_cancel_redemption;
use crate::execute::redemption::finalize_redemption::execute_finalize_redemption;
use crate::execute::redemption::initiate_redemption::execute_initiate_redemption;
use crate::execute::redemption::pool_redemption::execute_pool_redemption;
use crate::instantiate::instantiate_contract::instantiate_contract;
use crate::msg::{ExecuteMsg, InstantiateContractMsg, QueryMsg};
use crate::query::query_config::query_configuration;
use crate::query::query_contribution::query_pending_contribution;
use crate::query::query_contribution::query_pending_contribution_by_reference;
use crate::query::query_contributor::query_contributor_for_scope;
use crate::query::query_redemption::query_pending_redemption;
use crate::query::query_redemption::query_pending_redemption_by_reference;
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
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::InitiateContribution {
            scope_uuids,
            mint_ldt_amount,
            contribution_ref,
        } => execute_initialize_contribution(
            deps,
            info,
            scope_uuids,
            mint_ldt_amount,
            contribution_ref,
        ),
        ExecuteMsg::FinalizeContribution {
            contribution_id,
            marker_denom,
        } => execute_finalize_contribution(deps, env, info, contribution_id, marker_denom),
        ExecuteMsg::CancelContribution { contribution_id } => {
            execute_cancel_contribution(deps, info, contribution_id)
        }
        ExecuteMsg::InitiateRedemption {
            burn_ldt_amount,
            scope_uuids,
            pool_denom,
            num_coins,
            redeemer_address,
            redemption_ref,
        } => execute_initiate_redemption(
            deps,
            info,
            env,
            burn_ldt_amount,
            scope_uuids,
            pool_denom,
            num_coins,
            redeemer_address,
            redemption_ref,
        ),
        ExecuteMsg::PoolRedemption {
            redemption_id,
            num_scopes_to_pool,
        } => execute_pool_redemption(deps, info, env, redemption_id, num_scopes_to_pool),
        ExecuteMsg::FinalizeRedemption { redemption_id } => {
            execute_finalize_redemption(deps, env, info, redemption_id)
        }
        ExecuteMsg::CancelRedemption { redemption_id } => {
            execute_cancel_redemption(deps, info, redemption_id)
        }
        ExecuteMsg::CleanupOrphanMarkers { marker_denoms } => {
            execute_cleanup_orphan_markers(deps, env, info, marker_denoms)
        }
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
        QueryMsg::GetConfiguration {} => Ok(to_json_binary(&query_configuration(deps)?)?),
        QueryMsg::GetContributorForScope { scope_uuid } => Ok(to_json_binary(
            &query_contributor_for_scope(deps, scope_uuid)?,
        )?),
    }
}
