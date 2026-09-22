use crate::{
    error::ContractError,
    execute::assert_no_funds,
    state::{CONFIGURATION, PENDING_REDEMPTIONS},
};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_complete_redemption_pool(
    deps: DepsMut,
    info: MessageInfo,
    redemption_id: u64,
    burn_ldt_amount: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Burn amount should be greater than zero
    if burn_ldt_amount == 0 {
        return Err(ContractError::InvalidBurnAmount);
    }

    let configuration = CONFIGURATION.load(deps.storage)?;

    // Only the OTC can complete pooling
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Load the pending redemption from storage
    let mut redemption = PENDING_REDEMPTIONS.load(deps.storage, redemption_id)?;

    // Terms are write-once. The redeemer sends their burn LDT on the strength of the amount
    // published here, so re-pricing a completed pool would let that amount change after the
    // redeemer had already decided to confirm. Correcting a mistake means cancelling the
    // redemption and initiating a new one.
    if redemption.pooling_complete {
        return Err(ContractError::RedemptionPoolingAlreadyComplete);
    }

    // Mark the redemption as having completed pooling and set the burn LDT amount
    redemption.pooling_complete = true;
    redemption.burn_ldt_amount = Some(burn_ldt_amount);
    PENDING_REDEMPTIONS.save(deps.storage, redemption_id, &redemption)?;

    Ok(Response::new()
        .add_attribute("method", "complete_redemption_pool")
        .add_attribute("redemption_id", redemption_id.to_string())
        .add_attribute("burn_ldt_amount", burn_ldt_amount.to_string())
        .add_attribute("pooling_complete", "true"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingRedemption, PENDING_REDEMPTIONS};
    use crate::testing::{TestCtx, POOL_DENOM};

    fn seed_redemption(ctx: &mut TestCtx) {
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
    }

    #[test]
    fn otc_sets_burn_amount_and_marks_pooling_complete() {
        let mut ctx = TestCtx::new();
        seed_redemption(&mut ctx);

        let otc_info = ctx.otc_info();
        execute_complete_redemption_pool(ctx.deps.as_mut(), otc_info, 0, 90).unwrap();

        let stored = PENDING_REDEMPTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert!(stored.pooling_complete);
        assert_eq!(stored.burn_ldt_amount, Some(90));
    }

    #[test]
    fn rejects_second_complete_call_so_terms_cannot_be_repriced() {
        let mut ctx = TestCtx::new();
        seed_redemption(&mut ctx);

        let otc_info = ctx.otc_info();
        execute_complete_redemption_pool(ctx.deps.as_mut(), otc_info, 0, 90).unwrap();

        let otc_info = ctx.otc_info();
        let err = execute_complete_redemption_pool(ctx.deps.as_mut(), otc_info, 0, 5).unwrap_err();
        assert!(matches!(
            err,
            ContractError::RedemptionPoolingAlreadyComplete
        ));

        let stored = PENDING_REDEMPTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.burn_ldt_amount, Some(90));
    }

    #[test]
    fn rejects_zero_burn() {
        let mut ctx = TestCtx::new();
        seed_redemption(&mut ctx);

        let otc_info = ctx.otc_info();
        let err = execute_complete_redemption_pool(ctx.deps.as_mut(), otc_info, 0, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidBurnAmount));
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        seed_redemption(&mut ctx);

        let stranger_info = ctx.stranger_info();
        let err =
            execute_complete_redemption_pool(ctx.deps.as_mut(), stranger_info, 0, 90).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_complete_redemption_pool(ctx.deps.as_mut(), info, 0, 90).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
