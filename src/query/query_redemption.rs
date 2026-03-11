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
        scope_uuids: redemption.scope_uuids,
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
