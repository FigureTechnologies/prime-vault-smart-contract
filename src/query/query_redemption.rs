use cosmwasm_std::Deps;

use crate::{error::ContractError, msg::GetRedemptionResponse, state::PENDING_REDEMPTIONS};

pub fn query_pending_redemption(
    deps: Deps,
    redemption_id: u64,
) -> Result<GetRedemptionResponse, ContractError> {
    let redemption = PENDING_REDEMPTIONS
        .may_load(deps.storage, redemption_id)?
        .ok_or_else(|| ContractError::RedemptionNotFound { redemption_id })?;
    let response = GetRedemptionResponse {
        redemption_id,
        burn_ldt_amount: redemption.burn_ldt_amount,
        pool_denom: redemption.pool_denom,
        pooling_complete: redemption.pooling_complete,
        redeemer_addr: redemption.redeemer_addr.to_string(),
    };

    Ok(response)
}

pub fn query_pending_redemption_by_reference(
    deps: Deps,
    redemption_ref: String,
) -> Result<GetRedemptionResponse, ContractError> {
    let redemption_id = crate::state::REDEMPTION_REFERENCES
        .may_load(deps.storage, redemption_ref.clone())?
        .ok_or_else(|| ContractError::RedemptionReferenceNotFound {
            redemption_ref: redemption_ref.clone(),
        })?;
    query_pending_redemption(deps, redemption_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingRedemption, PENDING_REDEMPTIONS, REDEMPTION_REFERENCES};
    use crate::testing::{TestCtx, POOL_DENOM};

    #[test]
    fn burn_amount_is_absent_until_pooling_is_completed() {
        let mut ctx = TestCtx::new();
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                1,
                &PendingRedemption {
                    burn_ldt_amount: None,
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: false,
                    redemption_ref: Some("redemption-ref".to_string()),
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();
        REDEMPTION_REFERENCES
            .save(&mut ctx.deps.storage, "redemption-ref".to_string(), &1)
            .unwrap();

        let by_id = query_pending_redemption(ctx.deps.as_ref(), 1).unwrap();
        assert!(by_id.burn_ldt_amount.is_none());
        assert!(!by_id.pooling_complete);
        assert_eq!(by_id.pool_denom, POOL_DENOM);
        assert_eq!(by_id.redeemer_addr, ctx.redeemer.to_string());

        let by_ref =
            query_pending_redemption_by_reference(ctx.deps.as_ref(), "redemption-ref".to_string())
                .unwrap();
        assert_eq!(by_ref.redemption_id, 1);
    }

    #[test]
    fn missing_id_and_reference_are_distinct_errors() {
        let ctx = TestCtx::new();
        assert!(matches!(
            query_pending_redemption(ctx.deps.as_ref(), 9).unwrap_err(),
            ContractError::RedemptionNotFound { redemption_id: 9 }
        ));
        assert!(matches!(
            query_pending_redemption_by_reference(ctx.deps.as_ref(), "missing".to_string())
                .unwrap_err(),
            ContractError::RedemptionReferenceNotFound { .. }
        ));
    }
}
