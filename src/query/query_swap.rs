use cosmwasm_std::Deps;

use crate::{error::ContractError, msg::GetSwapResponse, state::PENDING_SWAPS};

pub fn query_pending_swap(deps: Deps, swap_id: u64) -> Result<GetSwapResponse, ContractError> {
    let swap = PENDING_SWAPS
        .may_load(deps.storage, swap_id)?
        .ok_or_else(|| ContractError::SwapNotFound { swap_id })?;
    Ok(GetSwapResponse {
        swap_id,
        pool_denom: swap.pool_denom,
        pooling_complete: swap.pooling_complete,
        contributor_addr: swap.contributor_addr.to_string(),
        mint_ldt_amount: swap.mint_ldt_amount,
        burn_ldt_amount: swap.burn_ldt_amount,
        incoming_marker_denom: swap.incoming_marker_denom,
    })
}

pub fn query_pending_swap_by_reference(
    deps: Deps,
    swap_ref: String,
) -> Result<GetSwapResponse, ContractError> {
    let swap_id = crate::state::SWAP_REFERENCES
        .may_load(deps.storage, swap_ref.clone())?
        .ok_or_else(|| ContractError::SwapReferenceNotFound {
            swap_ref: swap_ref.clone(),
        })?;
    query_pending_swap(deps, swap_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingSwap, PENDING_SWAPS, SWAP_REFERENCES};
    use crate::testing::{TestCtx, MARKER_DENOM, POOL_DENOM};

    #[test]
    fn returns_mint_and_burn_amounts_after_pool_completion() {
        let mut ctx = TestCtx::new();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                2,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: Some("swap-ref".to_string()),
                    mint_ldt_amount: 15,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: None,
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        SWAP_REFERENCES
            .save(&mut ctx.deps.storage, "swap-ref".to_string(), &2)
            .unwrap();

        let by_id = query_pending_swap(ctx.deps.as_ref(), 2).unwrap();
        assert!(by_id.pooling_complete);
        assert_eq!(by_id.pool_denom, POOL_DENOM);
        assert_eq!(by_id.mint_ldt_amount, 15);
        assert_eq!(by_id.burn_ldt_amount, 0);
        assert_eq!(by_id.contributor_addr, ctx.contributor.to_string());
        assert!(by_id.incoming_marker_denom.is_none());

        let by_ref =
            query_pending_swap_by_reference(ctx.deps.as_ref(), "swap-ref".to_string()).unwrap();
        assert_eq!(by_ref.swap_id, 2);
        assert_eq!(by_ref.mint_ldt_amount, 15);
        assert!(by_ref.incoming_marker_denom.is_none());
    }

    #[test]
    fn returns_incoming_marker_denom_after_submit() {
        let mut ctx = TestCtx::new();
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                3,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete: true,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: Some("submitted-ref".to_string()),
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 40,
                    incoming_marker_denom: Some(MARKER_DENOM.to_string()),
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        SWAP_REFERENCES
            .save(&mut ctx.deps.storage, "submitted-ref".to_string(), &3)
            .unwrap();

        let by_id = query_pending_swap(ctx.deps.as_ref(), 3).unwrap();
        assert!(by_id.pooling_complete);
        assert_eq!(by_id.incoming_marker_denom.as_deref(), Some(MARKER_DENOM));

        let by_ref =
            query_pending_swap_by_reference(ctx.deps.as_ref(), "submitted-ref".to_string())
                .unwrap();
        assert_eq!(by_ref.swap_id, 3);
        assert_eq!(by_ref.incoming_marker_denom, by_id.incoming_marker_denom);
    }

    #[test]
    fn missing_id_and_reference_are_distinct_errors() {
        let ctx = TestCtx::new();
        assert!(matches!(
            query_pending_swap(ctx.deps.as_ref(), 9).unwrap_err(),
            ContractError::SwapNotFound { swap_id: 9 }
        ));
        assert!(matches!(
            query_pending_swap_by_reference(ctx.deps.as_ref(), "missing".to_string()).unwrap_err(),
            ContractError::SwapReferenceNotFound { .. }
        ));
    }
}
