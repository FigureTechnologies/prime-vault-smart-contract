use crate::error::ContractError;
use crate::state::{CONFIGURATION, PENDING_REDEMPTIONS, REDEMPTION_REFERENCES};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_cancel_redemption(
    deps: DepsMut,
    info: MessageInfo,
    redemption_id: u64,
) -> Result<Response, ContractError> {
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
    PENDING_REDEMPTIONS.remove(deps.storage, redemption_id);
    if let Some(ref redemption_ref) = redemption.redemption_ref {
        REDEMPTION_REFERENCES.remove(deps.storage, redemption_ref.to_string());
    }

    Ok(Response::new()
        .add_attribute("redemption_id", redemption_id.to_string())
        .add_attribute("method", "cancel_redemption"))
}
