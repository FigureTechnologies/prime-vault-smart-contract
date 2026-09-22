use crate::{
    error::ContractError,
    execute::assert_no_funds,
    state::{CONFIGURATION, PENDING_SWAPS},
};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_complete_swap_pool(
    deps: DepsMut,
    info: MessageInfo,
    swap_id: u64,
    mint_ldt_amount: u64,
    burn_ldt_amount: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Load the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Verify that the sender is the authorized OTC address
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Ensure that both mint and burn amounts are not set simultaneously
    if mint_ldt_amount > 0 && burn_ldt_amount > 0 {
        return Err(ContractError::ConflictingSwapTokenAmounts);
    }

    // Load the pending swap
    let mut swap = PENDING_SWAPS.load(deps.storage, swap_id)?;

    // Terms are write-once. The contributor commits their incoming marker (and any burn LDT)
    // on the strength of the mint/burn amounts published here, so re-pricing a completed pool
    // would let those amounts change after the contributor had already decided to submit.
    // Correcting a mistake means cancelling the swap and initiating a new one.
    if swap.pooling_complete {
        return Err(ContractError::SwapPoolingAlreadyComplete);
    }
    if swap.incoming_marker_denom.is_some() {
        return Err(ContractError::SwapIncomingDenomAlreadySet);
    }

    // Mark the swap as complete and record the mint and burn amounts
    swap.pooling_complete = true;
    swap.mint_ldt_amount = mint_ldt_amount;
    swap.burn_ldt_amount = burn_ldt_amount;
    PENDING_SWAPS.save(deps.storage, swap_id, &swap)?;

    Ok(Response::new()
        .add_attribute("method", "complete_swap_pool")
        .add_attribute("swap_id", swap_id.to_string())
        .add_attribute("mint_ldt_amount", mint_ldt_amount.to_string())
        .add_attribute("burn_ldt_amount", burn_ldt_amount.to_string())
        .add_attribute("pooling_complete", "true"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingSwap, PENDING_SWAPS};
    use crate::testing::{TestCtx, POOL_DENOM};

    fn seed_swap(ctx: &mut TestCtx, incoming: Option<String>) {
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
                    incoming_marker_denom: incoming,
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
    }

    #[test]
    fn otc_records_mint_amount_and_marks_pooling_complete() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let otc_info = ctx.otc_info();
        execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 80, 0).unwrap();

        let swap = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert!(swap.pooling_complete);
        assert_eq!(swap.mint_ldt_amount, 80);
        assert_eq!(swap.burn_ldt_amount, 0);
    }

    #[test]
    fn otc_records_burn_amount() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let otc_info = ctx.otc_info();
        execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 0, 40).unwrap();

        let swap = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert!(swap.pooling_complete);
        assert_eq!(swap.mint_ldt_amount, 0);
        assert_eq!(swap.burn_ldt_amount, 40);
    }

    #[test]
    fn allows_zero_mint_and_zero_burn() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let otc_info = ctx.otc_info();
        execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 0, 0).unwrap();

        let swap = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert!(swap.pooling_complete);
        assert_eq!(swap.mint_ldt_amount, 0);
        assert_eq!(swap.burn_ldt_amount, 0);
    }

    #[test]
    fn rejects_mint_and_burn_together() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let otc_info = ctx.otc_info();
        let err = execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 10, 5).unwrap_err();
        assert!(matches!(err, ContractError::ConflictingSwapTokenAmounts));
    }

    #[test]
    fn rejects_second_complete_call_so_terms_cannot_be_repriced() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let otc_info = ctx.otc_info();
        execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 100, 0).unwrap();

        let otc_info = ctx.otc_info();
        let err = execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 0, 0).unwrap_err();
        assert!(matches!(err, ContractError::SwapPoolingAlreadyComplete));

        let swap = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(swap.mint_ldt_amount, 100);
    }

    #[test]
    fn rejects_complete_after_incoming_assets_submitted() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, Some("incoming.marker".to_string()));

        let otc_info = ctx.otc_info();
        let err = execute_complete_swap_pool(ctx.deps.as_mut(), otc_info, 0, 10, 0).unwrap_err();
        assert!(matches!(err, ContractError::SwapIncomingDenomAlreadySet));
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, None);

        let stranger_info = ctx.stranger_info();
        let err =
            execute_complete_swap_pool(ctx.deps.as_mut(), stranger_info, 0, 10, 0).unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_complete_swap_pool(ctx.deps.as_mut(), info, 0, 10, 0).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
