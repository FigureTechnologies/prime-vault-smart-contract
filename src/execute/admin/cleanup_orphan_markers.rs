use std::collections::BTreeSet;

use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::state::{PendingDenomOwner, CONFIGURATION, PENDING_DENOMS};
use crate::util::get_marker_by_denom;
use cosmwasm_std::{CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, Uint128};
use provwasm_std::types::cosmos::base::query::v1beta1::PageRequest;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    Access, AccessGrant, MsgAddAccessRequest, MsgCancelRequest, MsgDeleteRequest,
    MsgTransferRequest,
};
use provwasm_std::types::provenance::metadata::v1::MetadataQuerier;

/// Deletes the given marker denoms when the contract holds those marker coins in bank, the marker
/// is not the metadata value owner of any scope, no pending contribution, redemption, or swap
/// references the denom, and the denom is not the configured LDT denom.
///
/// For each denom: grant `ACCESS_DELETE` to the contract, transfer held marker coins back to the
/// marker base account via the marker module, then delete the marker. Duplicate denoms in the input
/// are de-duplicated; unique denoms are processed in ascending lexicographic order.
///
/// Per-denom work is split out of this function on purpose. CosmWasm rejects any Wasm function
/// with more than 100 locals; a single inlined body compiled to 87 locals, and `wasm-opt -Os`
/// (which inlines every single-caller function) inflated that to 103.
pub fn execute_cleanup_orphan_markers(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    marker_denoms: Vec<String>,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this admin action
    assert_no_funds(&info)?;
    require_contract_admin(deps.as_ref(), &env, &info)?;

    // The list of denoms to clean up should not be empty
    if marker_denoms.is_empty() {
        return Err(ContractError::EmptyOrphanMarkerDenoms);
    }
    // De-duplicate and sort the list of denoms to clean up
    let unique_denoms: Vec<String> = marker_denoms
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let ldt_denom = CONFIGURATION.load(deps.storage)?.ldt_denom;
    let contract_address = env.contract.address;

    let mut cleaned: Vec<(String, Uint128, String)> = Vec::new();
    for denom in &unique_denoms {
        cleaned.push(collect_orphan_marker(
            deps.as_ref(),
            denom,
            &ldt_denom,
            &contract_address,
        )?);
    }

    let mut messages: Vec<CosmosMsg> = Vec::new();
    for (denom, amount, marker_base_address) in &cleaned {
        messages.extend(orphan_delete_messages(
            contract_address.as_str(),
            denom,
            amount,
            marker_base_address,
        ));
    }

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("method", "cleanup_orphan_markers")
        .add_attribute("deleted_count", cleaned.len().to_string()))
}

fn require_contract_admin(deps: Deps, env: &Env, info: &MessageInfo) -> Result<(), ContractError> {
    let contract_info = deps
        .querier
        .query_wasm_contract_info(env.contract.address.clone())?;
    let admin = contract_info
        .admin
        .ok_or(ContractError::UnauthorizedContractAdmin)?;
    if info.sender != admin {
        return Err(ContractError::UnauthorizedContractAdmin);
    }
    Ok(())
}

/// Validates one denom is a contract-held orphan marker and returns `(denom, amount, marker base)`.
#[inline(never)]
fn collect_orphan_marker(
    deps: Deps,
    denom: &str,
    ldt_denom: &str,
    contract_address: &cosmwasm_std::Addr,
) -> Result<(String, Uint128, String), ContractError> {
    // We should not be targeting the LDT denom for cleanup
    if denom == ldt_denom {
        return Err(ContractError::CleanupLdtDenomNotAllowed);
    }

    // Query the marker by its denom and get the base account address
    let marker = get_marker_by_denom(deps, denom.to_string())?;
    let marker_base_address = marker
        .base_account
        .ok_or_else(|| ContractError::MarkerQueryError("missing marker base account".to_string()))?
        .address;

    // Make sure the contract balance isn't 0
    let balance = deps.querier.query_balance(contract_address, denom)?;
    if balance.amount.is_zero() {
        return Err(ContractError::OrphanMarkerNotHeldByContract {
            denom: denom.to_string(),
        });
    }

    // The marker owns any scope, we can't clean it up
    if marker_owns_any_scope(deps, denom.to_string())? {
        return Err(ContractError::OrphanMarkerStillOwnsScopes {
            denom: denom.to_string(),
        });
    }
    // If the marker is referenced by any pending denoms, we can't clean it up
    if let Some(owner) = PENDING_DENOMS.may_load(deps.storage, denom.to_string())? {
        return Err(pending_orphan_error(denom, owner));
    }

    Ok((denom.to_string(), balance.amount, marker_base_address))
}

fn pending_orphan_error(denom: &str, owner: PendingDenomOwner) -> ContractError {
    match owner {
        PendingDenomOwner::Contribution { id } => {
            ContractError::OrphanMarkerReferencedByPendingContribution {
                denom: denom.to_string(),
                contribution_id: id,
            }
        }
        PendingDenomOwner::Redemption { id } => {
            ContractError::OrphanMarkerReferencedByPendingRedemption {
                denom: denom.to_string(),
                redemption_id: id,
            }
        }
        PendingDenomOwner::Swap { id } => ContractError::OrphanMarkerReferencedByPendingSwap {
            denom: denom.to_string(),
            swap_id: id,
        },
    }
}

/// Grant delete access, return coins to the marker account, cancel, then delete.
#[inline(never)]
fn orphan_delete_messages(
    contract_address: &str,
    denom: &str,
    amount: &Uint128,
    marker_base_address: &str,
) -> Vec<CosmosMsg> {
    vec![
        // Delete is never granted as part of the standard flows
        CosmosMsg::from(MsgAddAccessRequest {
            denom: denom.to_string(),
            administrator: contract_address.to_string(),
            access: vec![AccessGrant {
                address: contract_address.to_string(),
                permissions: vec![
                    Access::Delete as i32,
                    Access::Deposit as i32,
                    Access::Transfer as i32,
                    Access::Admin as i32,
                ],
            }],
        }),
        CosmosMsg::from(MsgTransferRequest {
            amount: Some(Coin {
                denom: denom.to_string(),
                amount: amount.to_string(),
            }),
            administrator: contract_address.to_string(),
            from_address: contract_address.to_string(),
            to_address: marker_base_address.to_string(),
        }),
        CosmosMsg::from(MsgCancelRequest {
            denom: denom.to_string(),
            administrator: contract_address.to_string(),
        }),
        CosmosMsg::from(MsgDeleteRequest {
            denom: denom.to_string(),
            administrator: contract_address.to_string(),
        }),
    ]
}

/// Whether the marker is still the metadata value owner of any scope.
///
/// Stops at the first non-empty page. The caller only needs a yes/no, and accumulating the full
/// set first meant a marker owning thousands of scopes burned gas proportional to all of them
/// just to be rejected — enough for one such denom to push an otherwise valid batch over the
/// block gas limit.
fn marker_owns_any_scope(deps: Deps, marker_denom: String) -> Result<bool, ContractError> {
    let metadata_querier = MetadataQuerier::new(&deps.querier);

    let marker = get_marker_by_denom(deps, marker_denom.clone())?;
    let marker_address = marker
        .base_account
        .ok_or_else(|| {
            ContractError::MarkerQueryError("Error getting base account for marker".to_string())
        })?
        .address;

    let mut key: Vec<u8> = vec![];

    loop {
        let page_request = PageRequest {
            key: key.clone(),
            offset: 0,
            limit: 50,
            count_total: false,
            reverse: false,
        };

        let response = metadata_querier
            .value_ownership(marker_address.clone(), false, Some(page_request))
            .map_err(|e| {
                ContractError::ScopeQueryError(format!("value ownership query failed: {}", e))
            })?;

        if !response.scope_uuids.is_empty() {
            return Ok(true);
        }

        let next_key = match response.pagination.and_then(|p| p.next_key) {
            Some(next_key) if !next_key.is_empty() => next_key,
            _ => return Ok(false),
        };

        // Don't trust the node to terminate the walk. A next_key that repeats would otherwise
        // spin until the query runs out of gas.
        if next_key == key {
            return Err(ContractError::ScopeQueryError(
                "value ownership pagination did not advance".to_string(),
            ));
        }
        key = next_key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingContribution, PendingDenomOwner, PendingRedemption, PendingSwap,
        PENDING_CONTRIBUTIONS, PENDING_REDEMPTIONS, PENDING_SWAPS,
    };
    use crate::testing::{attr, typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM, POOL_DENOM};
    use provwasm_std::types::provenance::marker::v1::{
        Access, MsgAddAccessRequest, MsgCancelRequest, MsgDeleteRequest, MsgTransferRequest,
    };

    #[test]
    fn rejects_non_admin() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            stranger_info,
            vec!["orphan.marker".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedContractAdmin));
    }

    #[test]
    fn rejects_empty_denom_list() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err =
            execute_cleanup_orphan_markers(ctx.deps.as_mut(), env, otc_info, vec![]).unwrap_err();
        assert!(matches!(err, ContractError::EmptyOrphanMarkerDenoms));
    }

    #[test]
    fn rejects_ldt_denom() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![LDT_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::CleanupLdtDenomNotAllowed));
    }

    #[test]
    fn rejects_denom_whose_marker_still_owns_scopes() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_value_ownership_with_scopes(vec!["a-scope-uuid".to_string()]);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::OrphanMarkerStillOwnsScopes { denom } if denom == POOL_DENOM
        ));
    }

    #[test]
    fn rejects_value_ownership_pagination_that_never_advances() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_value_ownership_stuck_pagination();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::ScopeQueryError(_)));
    }

    #[test]
    fn rejects_denom_used_by_pending_redemption() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                7,
                &PendingRedemption {
                    burn_ldt_amount: None,
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: false,
                    redemption_ref: None,
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Redemption { id: 7 },
        )
        .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::OrphanMarkerReferencedByPendingRedemption {
                redemption_id: 7,
                ..
            }
        ));
    }

    #[test]
    fn rejects_denom_used_by_pending_swap() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                3,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: false,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: None,
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: None,
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Swap { id: 3 },
        )
        .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::OrphanMarkerReferencedByPendingSwap { swap_id: 3, .. }
        ));
    }

    #[test]
    fn rejects_denom_used_as_pending_contribution_marker() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(MARKER_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(MARKER_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                4,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: None,
                    marker_denom: Some(MARKER_DENOM.to_string()),
                    mint_ldt_amount: None,
                    stored_access_grants: None,
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            MARKER_DENOM,
            PendingDenomOwner::Contribution { id: 4 },
        )
        .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![MARKER_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::OrphanMarkerReferencedByPendingContribution {
                contribution_id: 4,
                ..
            }
        ));
    }

    #[test]
    fn rejects_denom_used_as_pending_swap_incoming_marker() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(MARKER_DENOM, 1);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(MARKER_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                5,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: false,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: None,
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: Some(MARKER_DENOM.to_string()),
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            MARKER_DENOM,
            PendingDenomOwner::Swap { id: 5 },
        )
        .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![MARKER_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::OrphanMarkerReferencedByPendingSwap { swap_id: 5, .. }
        ));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            info,
            vec!["orphan.marker".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn admin_deletes_an_orphan_the_contract_holds() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 4);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap();

        assert_eq!(attr(&res, "method"), "cleanup_orphan_markers");
        assert_eq!(attr(&res, "deleted_count"), "1");

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].denom, POOL_DENOM);
        assert_eq!(grants[0].administrator, contract);
        assert!(grants[0].access[0]
            .permissions
            .contains(&(Access::Delete as i32)));

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(transfers.len(), 1);
        assert_eq!(
            transfers[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((POOL_DENOM, "4"))
        );
        assert_eq!(transfers[0].from_address, contract);
        assert_eq!(
            transfers[0].to_address,
            format!("{POOL_DENOM}.marker.account")
        );

        let cancels: Vec<MsgCancelRequest> = typed_msgs(&res, MsgCancelRequest::TYPE_URL);
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].denom, POOL_DENOM);
        let deletes: Vec<MsgDeleteRequest> = typed_msgs(&res, MsgDeleteRequest::TYPE_URL);
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].denom, POOL_DENOM);

        let type_urls: Vec<&str> = res
            .messages
            .iter()
            .map(|msg| match &msg.msg {
                CosmosMsg::Any(any) => any.type_url.as_str(),
                other => panic!("expected stargate message, got {other:?}"),
            })
            .collect();
        assert_eq!(
            type_urls,
            vec![
                MsgAddAccessRequest::TYPE_URL,
                MsgTransferRequest::TYPE_URL,
                MsgCancelRequest::TYPE_URL,
                MsgDeleteRequest::TYPE_URL,
            ]
        );
    }

    #[test]
    fn dedupes_denoms_and_deletes_them_in_lexicographic_order() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balances(&[(POOL_DENOM, 4), (MARKER_DENOM, 9)]);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);
        ctx.mock_restricted_marker(MARKER_DENOM, &contract, "1", vec![]);
        ctx.mock_empty_value_ownership();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![
                POOL_DENOM.to_string(),
                MARKER_DENOM.to_string(),
                POOL_DENOM.to_string(),
            ],
        )
        .unwrap();

        assert_eq!(attr(&res, "deleted_count"), "2");
        let deletes: Vec<MsgDeleteRequest> = typed_msgs(&res, MsgDeleteRequest::TYPE_URL);
        assert_eq!(
            deletes
                .iter()
                .map(|msg| msg.denom.as_str())
                .collect::<Vec<_>>(),
            vec![MARKER_DENOM, POOL_DENOM]
        );
    }

    #[test]
    fn rejects_denom_the_contract_does_not_hold() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_admin(ctx.otc.clone());
        ctx.set_contract_bank_balance(POOL_DENOM, 0);
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(POOL_DENOM, &contract, "1", vec![]);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::OrphanMarkerNotHeldByContract { denom } if denom == POOL_DENOM
        ));
    }

    #[test]
    fn rejects_when_the_contract_has_no_admin() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_without_admin();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_cleanup_orphan_markers(
            ctx.deps.as_mut(),
            env,
            otc_info,
            vec![POOL_DENOM.to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedContractAdmin));
    }
}
