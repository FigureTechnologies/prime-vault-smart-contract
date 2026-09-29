use std::collections::HashSet;

use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::msg::PiClawback;
use crate::state::{normalize_settlement_ref, CompletedSettlement, CONFIGURATION, SETTLEMENTS};
use crate::util::get_marker_by_denom;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    Access, AccessGrant, MarkerStatus, MarkerType, MsgBurnRequest, MsgTransferRequest,
};

pub fn execute_settle_collection(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    settlement_ref: String,
    clawbacks: Vec<PiClawback>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;

    let configuration = CONFIGURATION.load(deps.storage)?;
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    let settlement_ref = normalize_settlement_ref(&settlement_ref)?;
    if SETTLEMENTS
        .may_load(deps.storage, settlement_ref.clone())?
        .is_some()
    {
        return Err(ContractError::SettlementAlreadyCompleted { settlement_ref });
    }
    if clawbacks.is_empty() {
        return Err(ContractError::InvalidClawbacks(
            "SettleCollection requires at least one clawback".to_string(),
        ));
    }

    let mut holders = HashSet::new();
    let mut total = Uint128::zero();
    let mut normalized: Vec<(String, Uint128)> = Vec::with_capacity(clawbacks.len());
    for clawback in clawbacks {
        let holder = deps.api.addr_validate(&clawback.holder_address)?;
        if clawback.pi_amount.is_zero() {
            return Err(ContractError::InvalidClawbacks(format!(
                "Clawback amount for {holder} must be positive"
            )));
        }
        if !holders.insert(holder.clone()) {
            return Err(ContractError::InvalidClawbacks(format!(
                "Duplicate clawback holder {holder}"
            )));
        }
        total = total
            .checked_add(clawback.pi_amount)
            .map_err(StdError::from)?;
        normalized.push((holder.to_string(), clawback.pi_amount));
    }

    let marker = get_marker_by_denom(deps.as_ref(), configuration.ldt_denom.clone())?;
    if marker.status != MarkerStatus::Active as i32 {
        return Err(ContractError::MarkerNotActive {
            status: marker.status,
        });
    }
    if marker.marker_type != MarkerType::Restricted as i32 {
        return Err(ContractError::MarkerNotRestricted {
            marker_type: marker.marker_type,
        });
    }
    if !marker.allow_forced_transfer {
        return Err(ContractError::PiMarkerForcedTransferDisabled);
    }
    let contract = env.contract.address.to_string();
    require_permission(&marker.access_control, &contract, Access::ForceTransfer)?;
    require_permission(&marker.access_control, &contract, Access::Burn)?;
    let marker_address = marker
        .base_account
        .ok_or_else(|| {
            ContractError::MarkerQueryError("Error getting base account for marker".to_string())
        })?
        .address;

    let mut response = Response::new();
    for (holder, amount) in &normalized {
        response = response.add_message(MsgTransferRequest {
            amount: Some(Coin {
                denom: configuration.ldt_denom.clone(),
                amount: amount.to_string(),
            }),
            administrator: contract.clone(),
            from_address: holder.clone(),
            to_address: marker_address.clone(),
        });
    }
    response = response.add_message(MsgBurnRequest {
        administrator: contract,
        amount: Some(Coin {
            denom: configuration.ldt_denom.clone(),
            amount: total.to_string(),
        }),
    });

    let holder_count = u32::try_from(normalized.len())
        .map_err(|_| ContractError::Std(StdError::generic_err("clawback count exceeds u32")))?;
    SETTLEMENTS.save(
        deps.storage,
        settlement_ref.clone(),
        &CompletedSettlement {
            total_pi_burned: total,
            holder_count,
        },
    )?;

    Ok(response
        .add_attribute("method", "settle_collection")
        .add_attribute("settlement_ref", settlement_ref)
        .add_attribute("total_pi_burned", total.to_string())
        .add_attribute("holder_count", holder_count.to_string()))
}

fn require_permission(
    access_control: &[AccessGrant],
    address: &str,
    permission: Access,
) -> Result<(), ContractError> {
    let grant = access_control.iter().find(|grant| grant.address == address);
    if grant.is_none_or(|grant| !grant.permissions.contains(&(permission as i32))) {
        return Err(ContractError::MissingMarkerPermission {
            permission: permission.as_str_name(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::GetSettlementResponse;
    use crate::query::query_settlement::query_settlement;
    use crate::state::MAX_SETTLEMENT_REF_LEN;
    use crate::testing::{contract_access, typed_msgs, TestCtx, LDT_DENOM};
    use cosmwasm_std::Coin as StdCoin;
    use provwasm_std::types::provenance::marker::v1::{
        Access, MarkerType, MsgBurnRequest, MsgTransferRequest,
    };

    fn clawback(holder: &str, amount: u128) -> PiClawback {
        PiClawback {
            holder_address: holder.to_string(),
            pi_amount: Uint128::new(amount),
        }
    }

    fn seed_pi_marker(ctx: &mut TestCtx, permissions: Vec<i32>, allow_forced_transfer: bool) {
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(
            LDT_DENOM,
            &ctx.contributor.to_string(),
            "100",
            vec![contract_access(&contract, permissions)],
        );
        ctx.set_marker_forced_transfer(LDT_DENOM, allow_forced_transfer);
    }

    fn ready_marker(ctx: &mut TestCtx) {
        seed_pi_marker(
            ctx,
            vec![Access::ForceTransfer as i32, Access::Burn as i32],
            true,
        );
    }

    fn settle(
        ctx: &mut TestCtx,
        info: cosmwasm_std::MessageInfo,
        settlement_ref: &str,
        clawbacks: Vec<PiClawback>,
    ) -> Result<Response, ContractError> {
        let env = ctx.env.clone();
        execute_settle_collection(
            ctx.deps.as_mut(),
            env,
            info,
            settlement_ref.to_string(),
            clawbacks,
        )
    }

    #[test]
    fn otc_force_transfers_each_holder_then_burns_the_sum() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder_a = ctx.contributor.to_string();
        let holder_b = ctx.redeemer.to_string();
        let marker_address = format!("{LDT_DENOM}.marker.account");
        let info = ctx.otc_info();

        let res = settle(
            &mut ctx,
            info,
            "settlement-1",
            vec![clawback(&holder_a, 40), clawback(&holder_b, 15)],
        )
        .unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .map(|attr| (attr.key.as_str(), attr.value.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("method", "settle_collection"),
                ("settlement_ref", "settlement-1"),
                ("total_pi_burned", "55"),
                ("holder_count", "2"),
            ]
        );

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(transfers.len(), 2);
        assert_eq!(transfers[0].administrator, ctx.contract_addr());
        assert_eq!(transfers[0].from_address, holder_a);
        assert_eq!(transfers[0].to_address, marker_address);
        assert_eq!(
            transfers[0]
                .amount
                .as_ref()
                .map(|coin| (coin.denom.as_str(), coin.amount.as_str())),
            Some((LDT_DENOM, "40"))
        );
        assert_eq!(transfers[1].from_address, holder_b);
        assert_eq!(
            transfers[1]
                .amount
                .as_ref()
                .map(|coin| coin.amount.as_str()),
            Some("15")
        );

        let burns: Vec<MsgBurnRequest> = typed_msgs(&res, MsgBurnRequest::TYPE_URL);
        assert_eq!(burns.len(), 1);
        assert_eq!(burns[0].administrator, ctx.contract_addr());
        assert_eq!(
            burns[0]
                .amount
                .as_ref()
                .map(|coin| (coin.denom.as_str(), coin.amount.as_str())),
            Some((LDT_DENOM, "55"))
        );
        assert!(res.messages.iter().all(|msg| matches!(
            &msg.msg,
            cosmwasm_std::CosmosMsg::Any(any)
                if any.type_url == MsgTransferRequest::TYPE_URL
                    || any.type_url == MsgBurnRequest::TYPE_URL
        )));

        assert_eq!(
            query_settlement(ctx.deps.as_ref(), "settlement-1".to_string()).unwrap(),
            GetSettlementResponse {
                settlement_ref: "settlement-1".to_string(),
                total_pi_burned: Uint128::new(55),
                holder_count: 2,
            }
        );
    }

    #[test]
    fn rejects_replay_without_emitting_another_burn() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info();
        settle(&mut ctx, info, "settlement-1", vec![clawback(&holder, 10)]).unwrap();

        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "settlement-1", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::SettlementAlreadyCompleted { settlement_ref } if settlement_ref == "settlement-1"
        ));

        for padded in [" settlement-1", "settlement-1 ", " settlement-1 "] {
            let info = ctx.otc_info();
            let err = settle(&mut ctx, info, padded, vec![clawback(&holder, 10)]).unwrap_err();
            assert!(matches!(
                err,
                ContractError::SettlementAlreadyCompleted { settlement_ref } if settlement_ref == "settlement-1"
            ));
        }

        assert_eq!(
            query_settlement(ctx.deps.as_ref(), " settlement-1 ".to_string()).unwrap(),
            GetSettlementResponse {
                settlement_ref: "settlement-1".to_string(),
                total_pi_burned: Uint128::new(10),
                holder_count: 1,
            }
        );
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder = ctx.contributor.to_string();
        let info = ctx.contributor_info();
        let err = settle(&mut ctx, info, "settlement-1", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_empty_zero_and_duplicate_clawbacks() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder = ctx.contributor.to_string();

        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "empty", vec![]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidClawbacks(ref reason) if reason.contains("at least one clawback")
        ));

        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "zero", vec![clawback(&holder, 0)]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidClawbacks(ref reason) if reason.contains("must be positive")
        ));

        let info = ctx.otc_info();
        let err = settle(
            &mut ctx,
            info,
            "duplicate",
            vec![clawback(&holder, 1), clawback(&holder, 2)],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidClawbacks(ref reason) if reason.contains("Duplicate clawback holder")
        ));
        assert!(SETTLEMENTS
            .may_load(&ctx.deps.storage, "duplicate".to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_marker_that_cannot_force_transfer() {
        let mut ctx = TestCtx::new();
        let contract = ctx.contract_addr();
        ctx.mock_marker(
            LDT_DENOM,
            &ctx.contributor.to_string(),
            "100",
            vec![contract_access(
                &contract,
                vec![Access::ForceTransfer as i32, Access::Burn as i32],
            )],
            MarkerStatus::Active,
            MarkerType::Coin,
        );
        ctx.set_marker_forced_transfer(LDT_DENOM, true);

        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "coin", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(err, ContractError::MarkerNotRestricted { .. }));

        let mut ctx = TestCtx::new();
        seed_pi_marker(
            &mut ctx,
            vec![Access::ForceTransfer as i32, Access::Burn as i32],
            false,
        );
        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "no-force", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(err, ContractError::PiMarkerForcedTransferDisabled));

        let mut ctx = TestCtx::new();
        seed_pi_marker(&mut ctx, vec![Access::Burn as i32], true);
        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "no-grant", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::MissingMarkerPermission {
                permission: "ACCESS_FORCE_TRANSFER"
            }
        ));

        let mut ctx = TestCtx::new();
        seed_pi_marker(&mut ctx, vec![Access::ForceTransfer as i32], true);
        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "no-burn", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::MissingMarkerPermission {
                permission: "ACCESS_BURN"
            }
        ));
    }

    #[test]
    fn rejects_bad_references_and_overflow() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder = ctx.contributor.to_string();
        let other = ctx.redeemer.to_string();

        let info = ctx.otc_info();
        let err = settle(&mut ctx, info, "   ", vec![clawback(&holder, 1)]).unwrap_err();
        assert!(matches!(err, ContractError::ReferenceEmpty));

        let info = ctx.otc_info();
        let err = settle(
            &mut ctx,
            info,
            &"s".repeat(MAX_SETTLEMENT_REF_LEN + 1),
            vec![clawback(&holder, 1)],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::ReferenceTooLong));

        let info = ctx.otc_info();
        let err = settle(
            &mut ctx,
            info,
            "overflow",
            vec![clawback(&holder, u128::MAX), clawback(&other, 1)],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));
        assert!(SETTLEMENTS
            .may_load(&ctx.deps.storage, "overflow".to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_attached_funds_and_does_not_record_the_settlement() {
        let mut ctx = TestCtx::new();
        ready_marker(&mut ctx);
        let holder = ctx.contributor.to_string();
        let info = ctx.otc_info_with_funds(&[StdCoin {
            denom: "ylds".to_string(),
            amount: Uint128::new(1),
        }]);
        let err = settle(&mut ctx, info, "funded", vec![clawback(&holder, 10)]).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
        assert!(SETTLEMENTS
            .may_load(&ctx.deps.storage, "funded".to_string())
            .unwrap()
            .is_none());
    }
}
