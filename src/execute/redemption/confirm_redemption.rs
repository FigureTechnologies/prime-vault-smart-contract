use crate::{
    error::ContractError,
    pending::unregister_pending_denom,
    state::{CONFIGURATION, PENDING_REDEMPTIONS, REDEMPTION_REFERENCES},
    util::{get_amount_holding_by_denom, get_marker_by_denom},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, Uint128};
use provwasm_std::types::{
    cosmos::bank::v1beta1::MsgSend,
    provenance::marker::v1::{
        Access, AccessGrant, MsgAddAccessRequest, MsgBurnRequest, MsgTransferRequest,
    },
};
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgDeleteAccessRequest,
};

pub fn execute_confirm_redemption(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    redemption_id: u64,
) -> Result<Response, ContractError> {
    // Verify that only one coin object was sent with the transaction
    if info.funds.len() != 1 {
        return Err(ContractError::InvalidFundsSent);
    }

    // Get the sent funds
    let coin = info.funds.first().ok_or(ContractError::InvalidFundsSent)?;

    // Load the redemption from storage
    let redemption = PENDING_REDEMPTIONS.load(deps.storage, redemption_id)?;

    // Make sure the sender is the redeemer specified in the redemption record
    if info.sender != redemption.redeemer_addr {
        return Err(ContractError::UnauthorizedRedeemer);
    }

    // Read the contract configuration to get the LDT denom
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Ensure the sent coin is in the correct denomination
    if coin.denom != configuration.ldt_denom {
        return Err(ContractError::InvalidCoinSent);
    }

    let burn_amount = redemption
        .burn_ldt_amount
        .ok_or(ContractError::MissingBurnAmount)?;

    // Make sure the sent amount matches the expected burn amount
    if Uint128::from(burn_amount) != coin.amount {
        return Err(ContractError::IncorrectTokenAmount);
    }

    // We can only confirm if the pooling is complete
    if !redemption.pooling_complete {
        return Err(ContractError::RedemptionPoolingIncomplete);
    }

    // The LDT tokens were sent to the contract, so now the contract can return them to the marker, then burn them.
    let ldt_marker = get_marker_by_denom(deps.as_ref(), configuration.ldt_denom.clone())?;
    let ldt_marker_base_account = ldt_marker.base_account.ok_or_else(|| {
        ContractError::MarkerQueryError("Error getting base account for marker".to_string())
    })?;

    // Send the ldt tokens back to the marker
    let msg_send_to_marker = MsgSend {
        from_address: env.contract.address.to_string(),
        to_address: ldt_marker_base_account.address.to_string(),
        amount: vec![Coin {
            denom: configuration.ldt_denom.to_string(),
            amount: coin.amount.to_string(),
        }],
    };

    // Burn the LDT tokens
    let msg_burn_ldt = MsgBurnRequest {
        administrator: env.contract.address.to_string(),
        amount: Some(Coin {
            denom: configuration.ldt_denom.to_string(),
            amount: coin.amount.to_string(),
        }),
    };

    // Transfer the scopes to the redeemer
    let pool_holding = get_amount_holding_by_denom(
        deps.as_ref(),
        redemption.pool_denom.clone(),
        env.contract.address.clone(),
    )?;
    let msg_transfer_scopes = MsgTransferRequest {
        amount: Some(Coin {
            denom: redemption.pool_denom.clone(),
            amount: pool_holding.amount,
        }),
        administrator: env.contract.address.to_string(),
        from_address: env.contract.address.to_string(),
        to_address: info.sender.to_string(),
    };

    // Set the redemption marker admin as the admin of the marker so they can manage the marker going forward.
    //The contract will remove its access in the next message.
    let msg_add_admin_access = MsgAddAccessRequest {
        denom: redemption.pool_denom.clone(),
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
    };

    // Remove the contract's admin access from the marker
    let msg_delete_access = MsgDeleteAccessRequest {
        denom: redemption.pool_denom.clone(),
        administrator: env.contract.address.to_string(),
        removed_address: env.contract.address.to_string(),
    };

    // Clear out the redemption record since it has been confirmed
    unregister_pending_denom(deps.storage, &redemption.pool_denom);
    PENDING_REDEMPTIONS.remove(deps.storage, redemption_id);
    if let Some(ref redemption_ref) = redemption.redemption_ref {
        REDEMPTION_REFERENCES.remove(deps.storage, redemption_ref.to_string());
    }

    Ok(Response::new()
        .add_message(msg_send_to_marker)
        .add_message(msg_burn_ldt)
        .add_message(msg_transfer_scopes)
        .add_message(msg_add_admin_access)
        .add_message(msg_delete_access)
        .add_attribute("method", "confirm_redemption"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingDenomOwner, PendingRedemption, PENDING_DENOMS, PENDING_REDEMPTIONS,
        REDEMPTION_REFERENCES,
    };
    use crate::testing::{contract_access, typed_msgs, TestCtx, LDT_DENOM, POOL_DENOM};
    use cosmwasm_std::{Coin as StdCoin, Uint128};
    use provwasm_std::types::cosmos::bank::v1beta1::MsgSend;
    use provwasm_std::types::provenance::marker::v1::{
        Access, MsgAddAccessRequest, MsgBurnRequest, MsgDeleteAccessRequest, MsgTransferRequest,
    };

    fn seed_ready_redemption(ctx: &mut TestCtx, pooling_complete: bool, burn: Option<u64>) {
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingRedemption {
                    burn_ldt_amount: burn,
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete,
                    redemption_ref: Some("redemption-ref".to_string()),
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();
        REDEMPTION_REFERENCES
            .save(&mut ctx.deps.storage, "redemption-ref".to_string(), &0)
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Redemption { id: 0 },
        )
        .unwrap();

        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(
            POOL_DENOM,
            &contract,
            "2",
            vec![contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            )],
        );
        ctx.mock_restricted_marker(
            LDT_DENOM,
            &contract,
            "0",
            vec![contract_access(&contract, vec![Access::Burn as i32])],
        );
    }

    fn ldt_funds(amount: u128) -> Vec<StdCoin> {
        vec![StdCoin {
            denom: LDT_DENOM.to_string(),
            amount: Uint128::new(amount),
        }]
    }

    #[test]
    fn redeemer_burns_ldt_and_receives_pooled_marker() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&ldt_funds(40));
        let res = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap();

        let sends: Vec<MsgSend> = typed_msgs(&res, MsgSend::TYPE_URL);
        assert!(sends
            .iter()
            .any(|msg| msg.from_address == ctx.contract_addr()));
        let burns: Vec<MsgBurnRequest> = typed_msgs(&res, MsgBurnRequest::TYPE_URL);
        assert_eq!(
            burns[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((LDT_DENOM, "40"))
        );
        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(transfers[0].to_address, ctx.redeemer.to_string());
        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert!(grants.iter().any(|msg| {
            msg.access
                .iter()
                .any(|g| g.address == ctx.redemption_admin.to_string())
        }));
        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes
            .iter()
            .any(|msg| msg.removed_address == ctx.contract_addr()));

        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
        assert!(REDEMPTION_REFERENCES
            .may_load(&ctx.deps.storage, "redemption-ref".to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_confirm_before_burn_amount_is_set() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, None);

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&ldt_funds(40));
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::MissingBurnAmount));
    }

    #[test]
    fn rejects_confirm_before_pooling_complete() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, false, Some(40));

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&ldt_funds(40));
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::RedemptionPoolingIncomplete));
    }

    #[test]
    fn rejects_wrong_ldt_amount() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&ldt_funds(1));
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::IncorrectTokenAmount));
    }

    #[test]
    fn rejects_wrong_denom() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&[StdCoin {
            denom: "not-ldt".to_string(),
            amount: Uint128::new(40),
        }]);
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidCoinSent));
    }

    #[test]
    fn rejects_non_redeemer() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&ldt_funds(40));
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedRedeemer));
    }

    #[test]
    fn rejects_missing_funds() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, otc_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_more_than_one_coin() {
        let mut ctx = TestCtx::new();
        seed_ready_redemption(&mut ctx, true, Some(40));

        let env = ctx.env.clone();
        let info = ctx.redeemer_info_with_funds(&[
            StdCoin {
                denom: LDT_DENOM.to_string(),
                amount: Uint128::new(40),
            },
            StdCoin {
                denom: "nhash".to_string(),
                amount: Uint128::new(1),
            },
        ]);
        let err = execute_confirm_redemption(ctx.deps.as_mut(), env, info, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_some());
    }
}
