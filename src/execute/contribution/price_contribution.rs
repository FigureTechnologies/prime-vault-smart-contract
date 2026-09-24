use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::state::{CONFIGURATION, PENDING_CONTRIBUTIONS};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

/// Records the OTC's LDT offer for a marker the contributor has already escrowed. Nothing is
/// minted or moved here; the offer only becomes binding once the contributor confirms it.
///
/// Unlike `complete_swap_pool`, re-pricing is deliberately allowed. The contributor escrowed
/// before any price existed, so they have committed nothing on the strength of a previous
/// figure, and correcting a mistake here does not require unwinding the marker escrow.
/// `confirm_contribution` pins the figure the contributor agreed to, so a re-price cannot be
/// used to settle at an amount they never saw.
pub fn execute_price_contribution(
    deps: DepsMut,
    info: MessageInfo,
    contribution_id: u64,
    mint_ldt_amount: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Load the current configuration from state
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the one pricing the contribution is the authorized OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Load the contribution from state
    let mut contribution = PENDING_CONTRIBUTIONS.load(deps.storage, contribution_id)?;

    // Pricing is only meaningful once there is an escrowed marker to price.
    if contribution.marker_denom.is_none() {
        return Err(ContractError::ContributionDenomNotSet);
    }

    // Save the mint ldt amount to the contribution
    contribution.mint_ldt_amount = Some(mint_ldt_amount);
    PENDING_CONTRIBUTIONS.save(deps.storage, contribution_id, &contribution)?;

    Ok(Response::new()
        .add_attribute("method", "price_contribution")
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("mint_ldt_amount", mint_ldt_amount.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingContribution, PENDING_CONTRIBUTIONS};
    use crate::testing::{attr, TestCtx, MARKER_DENOM};

    fn seed(ctx: &mut TestCtx, marker_denom: Option<String>, price: Option<u64>) {
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: None,
                    marker_denom,
                    mint_ldt_amount: price,
                    stored_access_grants: None,
                },
            )
            .unwrap();
    }

    #[test]
    fn otc_records_offer_without_moving_anything() {
        let mut ctx = TestCtx::new();
        seed(&mut ctx, Some(MARKER_DENOM.to_string()), None);

        let otc_info = ctx.otc_info();
        let res = execute_price_contribution(ctx.deps.as_mut(), otc_info, 0, 250).unwrap();

        assert_eq!(attr(&res, "method"), "price_contribution");
        assert_eq!(attr(&res, "mint_ldt_amount"), "250");
        assert!(res.messages.is_empty());
        assert_eq!(
            PENDING_CONTRIBUTIONS
                .load(&ctx.deps.storage, 0)
                .unwrap()
                .mint_ldt_amount,
            Some(250)
        );
    }

    #[test]
    fn allows_repricing_before_confirm() {
        let mut ctx = TestCtx::new();
        seed(&mut ctx, Some(MARKER_DENOM.to_string()), Some(250));

        let otc_info = ctx.otc_info();
        execute_price_contribution(ctx.deps.as_mut(), otc_info, 0, 300).unwrap();

        assert_eq!(
            PENDING_CONTRIBUTIONS
                .load(&ctx.deps.storage, 0)
                .unwrap()
                .mint_ldt_amount,
            Some(300)
        );
    }

    #[test]
    fn allows_zero_offer() {
        let mut ctx = TestCtx::new();
        seed(&mut ctx, Some(MARKER_DENOM.to_string()), None);

        let otc_info = ctx.otc_info();
        execute_price_contribution(ctx.deps.as_mut(), otc_info, 0, 0).unwrap();

        assert_eq!(
            PENDING_CONTRIBUTIONS
                .load(&ctx.deps.storage, 0)
                .unwrap()
                .mint_ldt_amount,
            Some(0)
        );
    }

    #[test]
    fn rejects_pricing_before_submit() {
        let mut ctx = TestCtx::new();
        seed(&mut ctx, None, None);

        let otc_info = ctx.otc_info();
        let err = execute_price_contribution(ctx.deps.as_mut(), otc_info, 0, 250).unwrap_err();

        assert!(matches!(err, ContractError::ContributionDenomNotSet));
        assert!(PENDING_CONTRIBUTIONS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .mint_ldt_amount
            .is_none());
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        seed(&mut ctx, Some(MARKER_DENOM.to_string()), None);

        let contributor_info = ctx.contributor_info();
        let err =
            execute_price_contribution(ctx.deps.as_mut(), contributor_info, 0, 250).unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_price_contribution(ctx.deps.as_mut(), info, 0, 250).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
