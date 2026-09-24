use crate::{
    error::ContractError,
    execute::assert_no_funds,
    pending::unregister_pending_denom,
    state::{CONFIGURATION, PENDING_SWAPS, SWAP_REFERENCES},
    util::{
        get_amount_holding_by_denom, get_marker_by_denom, require_marker_permissions,
        resolve_stored_marker_holding, CONTRACT_CUSTODY_PERMISSIONS,
    },
};
use cosmwasm_std::{CosmosMsg, DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::{
    cosmos::bank::v1beta1::MsgSend,
    provenance::marker::v1::{
        Access, AccessGrant, MsgAddAccessRequest, MsgBurnRequest, MsgDeleteAccessRequest,
        MsgMintRequest, MsgTransferRequest, MsgWithdrawRequest,
    },
};

pub fn execute_confirm_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    swap_id: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Load the contract configuration and the pending swap
    let configuration = CONFIGURATION.load(deps.storage)?;
    let swap = PENDING_SWAPS.load(deps.storage, swap_id)?;

    // Verify that the sender is the authorized OTC address
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Ensure that the swap pooling is complete before confirming
    if !swap.pooling_complete {
        return Err(ContractError::SwapPoolingIncomplete);
    }

    // Ensure that the incoming marker denom is set for this swap
    let incoming_marker_denom = swap
        .incoming_marker_denom
        .clone()
        .ok_or(ContractError::SwapIncomingDenomNotSet)?;

    // Read the incoming marker and balance
    let (incoming_marker, incoming_holding) =
        resolve_stored_marker_holding(deps.as_ref(), incoming_marker_denom.clone())?;

    // Refuse to mint until the queried grant already includes Admin, Withdraw, Deposit, and
    // Transfer. Submit grants this set, but confirm must not mint against a Transfer-only
    // (or otherwise incomplete) marker if that grant never landed.
    require_marker_permissions(
        &incoming_marker.access_control,
        env.contract.address.as_str(),
        &CONTRACT_CUSTODY_PERMISSIONS,
    )?;

    // Unregister the pending denoms and remove the swap from storage
    unregister_pending_denom(deps.storage, &swap.pool_denom);
    unregister_pending_denom(deps.storage, &incoming_marker_denom);
    PENDING_SWAPS.remove(deps.storage, swap_id);
    if let Some(ref swap_ref) = swap.swap_ref {
        SWAP_REFERENCES.remove(deps.storage, swap_ref.to_string());
    }

    let mut messages: Vec<CosmosMsg> = vec![];

    // If we are burning tokens, send them to the marker and then burn them
    if swap.burn_ldt_amount > 0 {
        let ldt_marker = get_marker_by_denom(deps.as_ref(), configuration.ldt_denom.clone())?;
        let ldt_marker_base_account = ldt_marker.base_account.ok_or_else(|| {
            ContractError::MarkerQueryError("Error getting base account for marker".to_string())
        })?;

        messages.push(CosmosMsg::from(MsgSend {
            from_address: env.contract.address.to_string(),
            to_address: ldt_marker_base_account.address.to_string(),
            amount: vec![Coin {
                denom: configuration.ldt_denom.to_string(),
                amount: swap.burn_ldt_amount.to_string(),
            }],
        }));

        messages.push(CosmosMsg::from(MsgBurnRequest {
            administrator: env.contract.address.to_string(),
            amount: Some(Coin {
                denom: configuration.ldt_denom.to_string(),
                amount: swap.burn_ldt_amount.to_string(),
            }),
        }));
    }

    // If we are minting tokens, mint them and withdraw them to the contributor
    if swap.mint_ldt_amount > 0 {
        messages.push(CosmosMsg::from(MsgMintRequest {
            administrator: env.contract.address.to_string(),
            amount: Option::Some(Coin {
                denom: configuration.ldt_denom.to_string(),
                amount: swap.mint_ldt_amount.to_string(),
            }),
        }));
        messages.push(CosmosMsg::from(MsgWithdrawRequest {
            denom: configuration.ldt_denom.to_string(),
            administrator: env.contract.address.to_string(),
            to_address: swap.contributor_addr.to_string(),
            amount: vec![Coin {
                denom: configuration.ldt_denom.to_string(),
                amount: swap.mint_ldt_amount.to_string(),
            }],
        }));
    }

    // Withdraw the coins from the incoming marker to the contract
    let msg_withdraw_incoming_marker = MsgWithdrawRequest {
        denom: incoming_marker_denom.clone(),
        administrator: env.contract.address.to_string(),
        to_address: env.contract.address.to_string(),
        amount: vec![Coin {
            denom: incoming_marker_denom.clone(),
            amount: incoming_holding.amount,
        }],
    };
    messages.push(CosmosMsg::from(msg_withdraw_incoming_marker));

    // Transfer the pool tokens from the contract to the contributor
    let pool_holding = get_amount_holding_by_denom(
        deps.as_ref(),
        swap.pool_denom.clone(),
        env.contract.address.clone(),
    )?;
    messages.push(CosmosMsg::from(MsgTransferRequest {
        amount: Some(Coin {
            denom: swap.pool_denom.clone(),
            amount: pool_holding.amount,
        }),
        administrator: env.contract.address.to_string(),
        from_address: env.contract.address.to_string(),
        to_address: swap.contributor_addr.to_string(),
    }));

    // Give the redemption marker admin full access to the pool tokens before removing the contract's access
    messages.push(CosmosMsg::from(MsgAddAccessRequest {
        denom: swap.pool_denom.clone(),
        administrator: env.contract.address.to_string(),
        access: vec![AccessGrant {
            address: configuration.redemption_marker_admin.to_string(),
            permissions: vec![
                Access::Admin as i32,
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ],
        }],
    }));

    // Remove the contract's access to the pool tokens after granting the redemption marker admin full access
    messages.push(CosmosMsg::from(MsgDeleteAccessRequest {
        denom: swap.pool_denom.clone(),
        administrator: env.contract.address.to_string(),
        removed_address: env.contract.address.to_string(),
    }));

    let response = Response::new()
        .add_messages(messages)
        .add_attribute("method", "confirm_swap")
        .add_attribute("swap_id", swap_id.to_string())
        .add_attribute("contributor", swap.contributor_addr.as_str())
        .add_attribute("incoming_marker_denom", incoming_marker_denom)
        .add_attribute("mint_ldt_amount", swap.mint_ldt_amount.to_string())
        .add_attribute("burn_ldt_amount", swap.burn_ldt_amount.to_string());

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingDenomOwner, PendingSwap, PENDING_DENOMS, PENDING_SWAPS, SWAP_REFERENCES,
    };
    use crate::testing::{
        contract_access, typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM, POOL_DENOM,
    };
    use crate::util::CONTRACT_CUSTODY_PERMISSIONS;
    use cosmwasm_std::{Coin as StdCoin, Uint128};
    use provwasm_std::types::cosmos::bank::v1beta1::MsgSend;
    use provwasm_std::types::provenance::marker::v1::{
        Access, MsgAddAccessRequest, MsgBurnRequest, MsgDeleteAccessRequest, MsgMintRequest,
        MsgTransferRequest, MsgWithdrawRequest,
    };

    fn seed_ready_swap(ctx: &mut TestCtx, mint: u64, burn: u64) {
        seed_ready_swap_with_permissions(ctx, mint, burn, CONTRACT_CUSTODY_PERMISSIONS.to_vec());
    }

    fn seed_ready_swap_with_permissions(
        ctx: &mut TestCtx,
        mint: u64,
        burn: u64,
        permissions: Vec<i32>,
    ) {
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: Some("swap-ref".to_string()),
                    mint_ldt_amount: mint,
                    burn_ldt_amount: burn,
                    incoming_marker_denom: Some(MARKER_DENOM.to_string()),
                    stored_incoming_access_grants: None,
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
        register_pending_denom(
            &mut ctx.deps.storage,
            MARKER_DENOM,
            PendingDenomOwner::Swap { id: 0 },
        )
        .unwrap();

        let contract = ctx.contract_addr();
        let incoming_account = format!("{MARKER_DENOM}.marker.account");
        ctx.mock_restricted_marker(
            MARKER_DENOM,
            &incoming_account,
            "1",
            vec![contract_access(&contract, permissions)],
        );
        ctx.mock_restricted_marker(
            POOL_DENOM,
            &contract,
            "3",
            vec![contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            )],
        );
        ctx.mock_restricted_marker(
            LDT_DENOM,
            &contract,
            "0",
            vec![contract_access(
                &contract,
                vec![
                    Access::Admin as i32,
                    Access::Burn as i32,
                    Access::Mint as i32,
                ],
            )],
        );
    }

    #[test]
    fn otc_mints_ldt_withdraws_incoming_and_hands_pool_to_contributor() {
        let mut ctx = TestCtx::new();
        seed_ready_swap(&mut ctx, 50, 0);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap();

        let mints: Vec<MsgMintRequest> = typed_msgs(&res, MsgMintRequest::TYPE_URL);
        assert_eq!(mints.len(), 1);
        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert!(withdraws.iter().any(|msg| {
            msg.denom == LDT_DENOM
                && msg.to_address == ctx.contributor.to_string()
                && msg
                    .amount
                    .iter()
                    .any(|c| c.denom == LDT_DENOM && c.amount == "50")
        }));
        assert!(withdraws
            .iter()
            .any(|msg| msg.denom == MARKER_DENOM && msg.to_address == ctx.contract_addr()));

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert!(transfers.iter().any(|msg| {
            msg.amount.as_ref().map(|c| c.denom.as_str()) == Some(POOL_DENOM)
                && msg.to_address == ctx.contributor.to_string()
        }));

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert!(grants.iter().all(|msg| msg.denom != MARKER_DENOM));
        assert!(grants.iter().any(|msg| {
            msg.denom == POOL_DENOM
                && msg
                    .access
                    .iter()
                    .any(|g| g.address == ctx.redemption_admin.to_string())
        }));
        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes
            .iter()
            .any(|msg| msg.denom == POOL_DENOM && msg.removed_address == ctx.contract_addr()));

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
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, MARKER_DENOM.to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn burns_ldt_already_paid_by_contributor() {
        let mut ctx = TestCtx::new();
        seed_ready_swap(&mut ctx, 0, 25);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap();

        let sends: Vec<MsgSend> = typed_msgs(&res, MsgSend::TYPE_URL);
        assert!(sends.iter().any(|msg| {
            msg.from_address == ctx.contract_addr()
                && msg
                    .amount
                    .iter()
                    .any(|c| c.denom == LDT_DENOM && c.amount == "25")
        }));
        let burns: Vec<MsgBurnRequest> = typed_msgs(&res, MsgBurnRequest::TYPE_URL);
        assert_eq!(burns.len(), 1);
        assert_eq!(
            burns[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((LDT_DENOM, "25"))
        );
        let mints: Vec<MsgMintRequest> = typed_msgs(&res, MsgMintRequest::TYPE_URL);
        assert!(mints.is_empty());
    }

    #[test]
    fn rejects_funds_on_confirm() {
        let mut ctx = TestCtx::new();
        seed_ready_swap(&mut ctx, 0, 25);

        let env = ctx.env.clone();
        let otc = ctx.otc.clone();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, message_info_with_ldt(&otc, 25), 0)
            .unwrap_err();

        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    fn message_info_with_ldt(sender: &cosmwasm_std::Addr, amount: u128) -> MessageInfo {
        cosmwasm_std::testing::message_info(
            sender,
            &[StdCoin {
                denom: LDT_DENOM.to_string(),
                amount: Uint128::new(amount),
            }],
        )
    }

    #[test]
    fn rejects_confirm_before_incoming_submit() {
        let mut ctx = TestCtx::new();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: None,
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: None,
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::SwapIncomingDenomNotSet));
    }

    #[test]
    fn rejects_confirm_before_pooling_complete() {
        let mut ctx = TestCtx::new();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
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

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::SwapPoolingIncomplete));
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        seed_ready_swap(&mut ctx, 0, 0);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, contributor_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_when_contract_has_only_transfer_access() {
        let mut ctx = TestCtx::new();
        seed_ready_swap_with_permissions(&mut ctx, 50, 0, vec![Access::Transfer as i32]);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();

        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }

    #[test]
    fn rejects_when_contract_lacks_admin_even_with_withdraw() {
        let mut ctx = TestCtx::new();
        seed_ready_swap_with_permissions(
            &mut ctx,
            50,
            0,
            vec![
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ],
        );

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();

        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
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
                    mint_ldt_amount: 50,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: Some(victim_address.clone()),
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        SWAP_REFERENCES
            .save(&mut ctx.deps.storage, "swap-ref".to_string(), &0)
            .unwrap();

        let contract = ctx.contract_addr();
        let contributor = ctx.contributor.to_string();
        ctx.mock_restricted_marker_with_address(
            MARKER_DENOM,
            &victim_address,
            &format!("{MARKER_DENOM}.marker.account"),
            "1",
            vec![contract_access(
                &contract,
                CONTRACT_CUSTODY_PERMISSIONS.to_vec(),
            )],
        );
        ctx.mock_restricted_marker(&victim_address, &contributor, "1", vec![]);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_swap(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();

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
