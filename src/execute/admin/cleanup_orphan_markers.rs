use std::collections::BTreeSet;

use crate::error::ContractError;
use crate::state::{CONFIGURATION, PENDING_REDEMPTIONS};
use crate::util::{get_all_owned_scopes, get_marker};
use cosmwasm_std::{CosmosMsg, DepsMut, Env, MessageInfo, Order, Response, StdResult, Storage, Uint128};
use cw_storage_plus::Bound;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    Access, AccessGrant, MsgAddAccessRequest, MsgCancelRequest, MsgDeleteRequest,
    MsgTransferRequest,
};

/// Deletes the given marker denoms when the contract holds those marker coins in bank, the marker
/// is not the metadata value owner of any scope, no pending redemption references the denom as
/// `pool_denom`, and the denom is not the configured LDT denom.
///
/// For each denom: grant `ACCESS_DELETE` to the contract, transfer held marker coins back to the
/// marker base account via the marker module, then delete the marker. Duplicate denoms in the input
/// are de-duplicated; unique denoms are processed in ascending lexicographic order.
pub fn execute_cleanup_orphan_markers(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    marker_denoms: Vec<String>,
) -> Result<Response, ContractError> {
    let contract_info = deps
        .querier
        .query_wasm_contract_info(env.contract.address.clone())?;
    let admin = contract_info
        .admin
        .ok_or(ContractError::UnauthorizedContractAdmin)?;
    if info.sender != admin {
        return Err(ContractError::UnauthorizedContractAdmin);
    }

    if marker_denoms.is_empty() {
        return Err(ContractError::EmptyOrphanMarkerDenoms);
    }

    let unique_denoms: Vec<String> = marker_denoms
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let configuration = CONFIGURATION.load(deps.storage)?;

    let mut cleaned: Vec<(String, Uint128, String)> = Vec::new();

    for denom in &unique_denoms {
        if denom.as_str() == configuration.ldt_denom.as_str() {
            return Err(ContractError::CleanupLdtDenomNotAllowed);
        }

        let marker = get_marker(deps.as_ref(), denom.clone())?;
        let marker_base_address = marker
            .base_account
            .ok_or_else(|| {
                ContractError::MarkerQueryError("missing marker base account".to_string())
            })?
            .address;

        let balance = deps
            .querier
            .query_balance(&env.contract.address, denom.as_str())?;
        if balance.amount.is_zero() {
            return Err(ContractError::OrphanMarkerNotHeldByContract {
                denom: denom.clone(),
            });
        }

        if !get_all_owned_scopes(deps.as_ref(), denom.clone())?.is_empty() {
            return Err(ContractError::OrphanMarkerStillOwnsScopes {
                denom: denom.clone(),
            });
        }

        if let Some(redemption_id) =
            first_pending_redemption_id_for_pool_denom(deps.storage, denom.as_str())?
        {
            return Err(ContractError::OrphanMarkerReferencedByPendingRedemption {
                denom: denom.clone(),
                redemption_id,
            });
        }

        cleaned.push((denom.clone(), balance.amount, marker_base_address));
    }

    let mut messages: Vec<CosmosMsg> = Vec::new();

    for (denom, amount, marker_base_address) in &cleaned {
        messages.push(CosmosMsg::from(MsgAddAccessRequest {
            denom: denom.clone(),
            administrator: env.contract.address.to_string(),
            access: vec![AccessGrant {
                address: env.contract.address.to_string(),
                permissions: vec![
                    Access::Delete as i32,
                    Access::Deposit as i32,
                    Access::Transfer as i32,
                    Access::Admin as i32,
                ],
            }],
        }));

        messages.push(CosmosMsg::from(MsgTransferRequest {
            amount: Some(Coin {
                denom: denom.clone(),
                amount: amount.to_string(),
            }),
            administrator: env.contract.address.to_string(),
            from_address: env.contract.address.to_string(),
            to_address: marker_base_address.clone(),
        }));

        messages.push(CosmosMsg::from(MsgCancelRequest {
            denom: denom.clone(),
            administrator: env.contract.address.to_string(),
        }));

        messages.push(CosmosMsg::from(MsgDeleteRequest {
            denom: denom.clone(),
            administrator: env.contract.address.to_string(),
        }));
    }

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("method", "cleanup_orphan_markers")
        .add_attribute("deleted_count", cleaned.len().to_string()))
}

/// Paginates `PENDING_REDEMPTIONS` so cleanup stays bounded for large maps.
fn first_pending_redemption_id_for_pool_denom(
    storage: &dyn Storage,
    pool_denom: &str,
) -> StdResult<Option<u64>> {
    let mut start_after = None;
    const PAGE: usize = 100;
    loop {
        let start = start_after.map(Bound::exclusive);
        let page: Vec<(u64, _)> = PENDING_REDEMPTIONS
            .range(storage, start, None, Order::Ascending)
            .take(PAGE)
            .collect::<StdResult<_>>()?;
        if page.is_empty() {
            return Ok(None);
        }
        if let Some((id, redemption)) = page
            .iter()
            .find(|(_, redemption)| redemption.pool_denom == pool_denom)
        {
            return Ok(Some(*id));
        }
        start_after = Some(page.last().unwrap().0);
    }
}
