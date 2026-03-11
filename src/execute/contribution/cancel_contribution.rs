use crate::error::ContractError;
use crate::state::{CONFIGURATION, CONTRIBUTION_REFERENCES, PENDING_CONTRIBUTIONS};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_cancel_contribution(
    deps: DepsMut,
    info: MessageInfo,
    contribution_id: u64,
) -> Result<Response, ContractError> {
    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // A pending contribution doesn't have any other clean up associated with it,
    // so we can just remove it from storage and be done.
    let contribution = PENDING_CONTRIBUTIONS.load(deps.storage, contribution_id)?;
    PENDING_CONTRIBUTIONS.remove(deps.storage, contribution_id);
    if let Some(ref contribution_ref) = contribution.contribution_ref {
        CONTRIBUTION_REFERENCES.remove(deps.storage, contribution_ref.to_string());
    }

    Ok(Response::new()
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("method", "cancel_contribution"))
}
