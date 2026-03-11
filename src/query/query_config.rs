use cosmwasm_std::Deps;

use crate::error::ContractError;
use crate::state::CONFIGURATION;

pub fn query_configuration(deps: Deps) -> Result<crate::state::Configuration, ContractError> {
    Ok(CONFIGURATION.load(deps.storage)?)
}
