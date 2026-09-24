use cosmwasm_std::Deps;

use crate::{error::ContractError, msg::GetContributionResponse, state::PENDING_CONTRIBUTIONS};

pub fn query_pending_contribution(
    deps: Deps,
    contribution_id: u64,
) -> Result<GetContributionResponse, ContractError> {
    let contribution = PENDING_CONTRIBUTIONS
        .may_load(deps.storage, contribution_id)?
        .ok_or_else(|| ContractError::ContributionNotFound { contribution_id })?;
    let response = GetContributionResponse {
        contribution_id,
        contributor_addr: contribution.contributor_addr.to_string(),
        marker_denom: contribution.marker_denom,
        mint_ldt_amount: contribution.mint_ldt_amount,
    };

    Ok(response)
}

pub fn query_pending_contribution_by_reference(
    deps: Deps,
    contribution_ref: String,
) -> Result<GetContributionResponse, ContractError> {
    let contribution_id = crate::state::CONTRIBUTION_REFERENCES
        .may_load(deps.storage, contribution_ref.clone())?
        .ok_or_else(|| ContractError::ContributionReferenceNotFound {
            contribution_ref: contribution_ref.clone(),
        })?;
    query_pending_contribution(deps, contribution_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingContribution, CONTRIBUTION_REFERENCES, PENDING_CONTRIBUTIONS};
    use crate::testing::{TestCtx, MARKER_DENOM};

    #[test]
    fn returns_pending_contribution_including_submitted_marker() {
        let mut ctx = TestCtx::new();
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                4,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: Some("contrib-ref".to_string()),
                    marker_denom: Some(MARKER_DENOM.to_string()),
                    mint_ldt_amount: Some(250),
                    stored_access_grants: None,
                },
            )
            .unwrap();
        CONTRIBUTION_REFERENCES
            .save(&mut ctx.deps.storage, "contrib-ref".to_string(), &4)
            .unwrap();

        let by_id = query_pending_contribution(ctx.deps.as_ref(), 4).unwrap();
        assert_eq!(by_id.contributor_addr, ctx.contributor.to_string());
        assert_eq!(by_id.marker_denom.as_deref(), Some(MARKER_DENOM));
        assert_eq!(by_id.mint_ldt_amount, Some(250));

        let by_ref =
            query_pending_contribution_by_reference(ctx.deps.as_ref(), "contrib-ref".to_string())
                .unwrap();
        assert_eq!(by_ref.contribution_id, 4);
        assert_eq!(by_ref.marker_denom, by_id.marker_denom);
    }

    #[test]
    fn returns_none_marker_and_price_before_submit() {
        let mut ctx = TestCtx::new();
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: None,
                    marker_denom: None,
                    mint_ldt_amount: None,
                    stored_access_grants: None,
                },
            )
            .unwrap();

        let res = query_pending_contribution(ctx.deps.as_ref(), 0).unwrap();
        assert!(res.marker_denom.is_none());
        assert!(res.mint_ldt_amount.is_none());
    }

    #[test]
    fn missing_id_and_reference_are_distinct_errors() {
        let ctx = TestCtx::new();
        assert!(matches!(
            query_pending_contribution(ctx.deps.as_ref(), 9).unwrap_err(),
            ContractError::ContributionNotFound { contribution_id: 9 }
        ));
        assert!(matches!(
            query_pending_contribution_by_reference(ctx.deps.as_ref(), "missing".to_string())
                .unwrap_err(),
            ContractError::ContributionReferenceNotFound { .. }
        ));
    }
}
