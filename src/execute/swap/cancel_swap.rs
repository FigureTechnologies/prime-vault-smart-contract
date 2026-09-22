use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::pending::unregister_pending_denom;
use crate::state::{CONFIGURATION, PENDING_SWAPS, SWAP_REFERENCES};
use crate::util::resolve_stored_marker_holding;
use cosmwasm_std::{CosmosMsg, DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::cosmos::bank::v1beta1::MsgSend;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    AccessGrant, MsgAddAccessRequest, MsgDeleteAccessRequest, MsgWithdrawRequest,
};

pub fn execute_cancel_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    swap_id: u64,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;

    let configuration = CONFIGURATION.load(deps.storage)?;
    let swap = PENDING_SWAPS.load(deps.storage, swap_id)?;

    // Make sure the sender is authorized as the OTC or contributor
    let is_otc = info.sender == configuration.otc_address;
    let is_contributor = info.sender == swap.contributor_addr;
    if !is_otc && !is_contributor {
        return Err(ContractError::UnauthorizedSwapCancel);
    }

    let mut messages: Vec<CosmosMsg> = vec![];

    if let Some(incoming_marker_denom) = swap.incoming_marker_denom.clone() {
        if swap.burn_ldt_amount > 0 {
            messages.push(CosmosMsg::from(MsgSend {
                from_address: env.contract.address.to_string(),
                to_address: swap.contributor_addr.to_string(),
                amount: vec![Coin {
                    denom: configuration.ldt_denom.to_string(),
                    amount: swap.burn_ldt_amount.to_string(),
                }],
            }));
        }

        let (_incoming_marker, incoming_balance) =
            resolve_stored_marker_holding(deps.as_ref(), incoming_marker_denom.clone())?;

        // Escrow lives in the marker's own account after submit. TransferCoin
        // cannot move it unless the contract is the sender, holds ForceTransfer,
        // or has an authz grant from the marker — none of which is true. Withdraw
        // is the same authorized path confirm_swap already uses.
        // Always emit the stored denom; never rebind to the resolved holding denom.
        messages.push(CosmosMsg::from(MsgWithdrawRequest {
            denom: incoming_marker_denom.clone(),
            administrator: env.contract.address.to_string(),
            to_address: swap.contributor_addr.to_string(),
            amount: vec![Coin {
                denom: incoming_marker_denom.clone(),
                amount: incoming_balance.amount,
            }],
        }));

        if let Some(stored_incoming_access_grants) = swap.stored_incoming_access_grants {
            for grant in stored_incoming_access_grants {
                messages.push(CosmosMsg::from(MsgAddAccessRequest {
                    denom: incoming_marker_denom.clone(),
                    administrator: env.contract.address.to_string(),
                    access: vec![AccessGrant {
                        address: grant.address,
                        permissions: grant.permissions,
                    }],
                }));
            }
        }

        // GrantAccess merges, so restoring the pre-submit snapshot would leave
        // the elevated custody set from submit. Delete the contract grant last.
        messages.push(CosmosMsg::from(MsgDeleteAccessRequest {
            denom: incoming_marker_denom.clone(),
            administrator: env.contract.address.to_string(),
            removed_address: env.contract.address.to_string(),
        }));
        unregister_pending_denom(deps.storage, incoming_marker_denom);
    }

    unregister_pending_denom(deps.storage, &swap.pool_denom);
    PENDING_SWAPS.remove(deps.storage, swap_id);
    if let Some(ref swap_ref) = swap.swap_ref {
        SWAP_REFERENCES.remove(deps.storage, swap_ref.to_string());
    }

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("swap_id", swap_id.to_string())
        .add_attribute("method", "cancel_swap"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingDenomOwner, PendingSwap, StoredAccessGrant, PENDING_DENOMS, PENDING_SWAPS,
        SWAP_REFERENCES,
    };
    use crate::testing::{typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM, POOL_DENOM};
    use provwasm_std::types::cosmos::bank::v1beta1::MsgSend;
    use provwasm_std::types::provenance::marker::v1::{
        Access, MsgAddAccessRequest, MsgDeleteAccessRequest, MsgTransferRequest, MsgWithdrawRequest,
    };

    fn seed_swap(ctx: &mut TestCtx, submitted: bool, burn: u64) {
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: Some("swap-ref".to_string()),
                    mint_ldt_amount: 0,
                    burn_ldt_amount: burn,
                    incoming_marker_denom: submitted.then(|| MARKER_DENOM.to_string()),
                    stored_incoming_access_grants: submitted.then(|| {
                        vec![StoredAccessGrant {
                            address: ctx.stranger.to_string(),
                            permissions: vec![Access::Admin as i32],
                        }]
                    }),
                },
            )
            .unwrap();
        SWAP_REFERENCES
            .save(&mut ctx.deps.storage, "swap-ref".to_string(), &0)
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Swap { id: 0 },
        )
        .unwrap();
        if submitted {
            register_pending_denom(
                &mut ctx.deps.storage,
                MARKER_DENOM,
                PendingDenomOwner::Swap { id: 0 },
            )
            .unwrap();
        }
    }

    #[test]
    fn otc_can_cancel_unsubmitted_swap() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, false, 0);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        execute_cancel_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap();

        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(SWAP_REFERENCES
            .may_load(&ctx.deps.storage, "swap-ref".to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn contributor_cancel_refunds_ldt_and_restores_incoming_marker() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 40);
        let incoming_account = format!("{MARKER_DENOM}.marker.account");
        ctx.mock_restricted_marker(MARKER_DENOM, &incoming_account, "1", vec![]);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_cancel_swap(ctx.deps.as_mut(), env, contributor_info, 0).unwrap();

        let refunds: Vec<MsgSend> = typed_msgs(&res, MsgSend::TYPE_URL);
        assert!(refunds.iter().any(|msg| {
            msg.to_address == ctx.contributor.to_string()
                && msg
                    .amount
                    .iter()
                    .any(|c| c.denom == LDT_DENOM && c.amount == "40")
        }));

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert!(transfers.is_empty());

        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws.len(), 1);
        assert_eq!(withdraws[0].denom, MARKER_DENOM);
        assert_eq!(withdraws[0].administrator, ctx.contract_addr());
        assert_eq!(withdraws[0].to_address, ctx.contributor.to_string());
        assert_eq!(
            withdraws[0].amount,
            vec![Coin {
                denom: MARKER_DENOM.to_string(),
                amount: "1".to_string(),
            }]
        );

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert!(grants.iter().any(|msg| {
            msg.access
                .iter()
                .any(|g| g.address == ctx.stranger.to_string())
        }));
        assert!(grants
            .iter()
            .all(|msg| { msg.access.iter().all(|g| g.address != ctx.contract_addr()) }));

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].denom, MARKER_DENOM);
        assert_eq!(deletes[0].administrator, ctx.contract_addr());
        assert_eq!(deletes[0].removed_address, ctx.contract_addr());
        match &res.messages.last().unwrap().msg {
            CosmosMsg::Any(any) => {
                assert_eq!(any.type_url, MsgDeleteAccessRequest::TYPE_URL);
            }
            other => panic!("expected MsgDeleteAccessRequest as last message, got {other:?}"),
        }
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, MARKER_DENOM.to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn does_not_refund_ldt_when_no_burn_was_paid() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0);
        let incoming_account = format!("{MARKER_DENOM}.marker.account");
        ctx.mock_restricted_marker(MARKER_DENOM, &incoming_account, "1", vec![]);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_cancel_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap();
        let refunds: Vec<MsgSend> = typed_msgs(&res, MsgSend::TYPE_URL);
        assert!(refunds.is_empty());
    }

    #[test]
    fn rejects_unrelated_address() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, false, 0);

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_cancel_swap(ctx.deps.as_mut(), env, stranger_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedSwapCancel));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_cancel_swap(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_when_stored_incoming_denom_resolves_to_a_different_marker() {
        let mut ctx = TestCtx::new();
        let victim_address = ctx.deps.api.addr_make("victim-marker").to_string();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: Some("swap-ref".to_string()),
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: Some(victim_address.clone()),
                    stored_incoming_access_grants: Some(vec![StoredAccessGrant {
                        address: ctx.stranger.to_string(),
                        permissions: vec![Access::Admin as i32],
                    }]),
                },
            )
            .unwrap();
        SWAP_REFERENCES
            .save(&mut ctx.deps.storage, "swap-ref".to_string(), &0)
            .unwrap();

        let contributor = ctx.contributor.to_string();
        ctx.mock_restricted_marker_with_address(
            MARKER_DENOM,
            &victim_address,
            &format!("{MARKER_DENOM}.marker.account"),
            "1",
            vec![],
        );
        ctx.mock_restricted_marker(&victim_address, &contributor, "1", vec![]);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_cancel_swap(ctx.deps.as_mut(), env, contributor_info, 0).unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomResolutionMismatch {
                requested_denom,
                resolved_denom,
            } if requested_denom == victim_address && resolved_denom == MARKER_DENOM
        ));
        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }
}
