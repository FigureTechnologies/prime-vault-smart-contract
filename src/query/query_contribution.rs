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
        mint_ldt_amount: contribution.mint_ldt_amount,
        scope_uuids: contribution.scope_uuids,
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
