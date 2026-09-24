use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::pending::unregister_pending_denom;
use crate::state::{CONFIGURATION, PENDING_REDEMPTIONS, REDEMPTION_REFERENCES};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_cancel_redemption(
    deps: DepsMut,
    info: MessageInfo,
    redemption_id: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // It is possible that a redemption could be partially pooled, but since that does not change
    // underlying ownership (just the marker) that owns it, we are safe to leave the scopes as they
    // are and remove the redemption from storage to effectively cancel the redemption.
    let redemption = PENDING_REDEMPTIONS.load(deps.storage, redemption_id)?;

    // Unregister the pending denom associated with this redemption and remove it from storage
    unregister_pending_denom(deps.storage, &redemption.pool_denom);

    // Remove the pending redemption from storage and clear any associated references
    PENDING_REDEMPTIONS.remove(deps.storage, redemption_id);
    if let Some(ref redemption_ref) = redemption.redemption_ref {
        REDEMPTION_REFERENCES.remove(deps.storage, redemption_ref.to_string());
    }

    Ok(Response::new()
        .add_attribute("redemption_id", redemption_id.to_string())
        .add_attribute("method", "cancel_redemption"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingDenomOwner, PendingRedemption, PENDING_DENOMS, PENDING_REDEMPTIONS,
        REDEMPTION_REFERENCES,
    };
    use crate::testing::{TestCtx, POOL_DENOM};

    #[test]
    fn otc_cancels_pending_redemption_and_clears_reference() {
        let mut ctx = TestCtx::new();
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingRedemption {
                    burn_ldt_amount: Some(10),
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
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

        let otc_info = ctx.otc_info();
        execute_cancel_redemption(ctx.deps.as_mut(), otc_info, 0).unwrap();

        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(REDEMPTION_REFERENCES
            .may_load(&ctx.deps.storage, "redemption-ref".to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingRedemption {
                    burn_ldt_amount: None,
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: false,
                    redemption_ref: None,
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();

        let stranger_info = ctx.stranger_info();
        let err = execute_cancel_redemption(ctx.deps.as_mut(), stranger_info, 0).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_cancel_redemption(ctx.deps.as_mut(), info, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
