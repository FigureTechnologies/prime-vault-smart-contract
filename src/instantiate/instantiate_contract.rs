use crate::error::ContractError;
use crate::msg::InstantiateContractMsg;
use crate::state::{CONFIGURATION, CONTRACT_INFO, CONTRACT_NAME, CONTRACT_VERSION};
use crate::util::create_marker_messages;
use cosmwasm_std::{DepsMut, Env, Response};
use cw2::ContractVersion;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{Access, AccessGrant, MarkerType};

pub fn instantiate_contract(
    deps: DepsMut,
    env: Env,
    msg: InstantiateContractMsg,
) -> Result<Response, ContractError> {
    let validated_otc_address = deps.api.addr_validate(&msg.otc_address)?;
    let validated_redemption_marker_admin = deps.api.addr_validate(&msg.redemption_marker_admin)?;
    let trust_address = deps.api.addr_validate(&msg.trust_address)?;

    // Store the contract configuration
    CONFIGURATION.save(
        deps.storage,
        &crate::state::Configuration {
            otc_address: validated_otc_address.clone(),
            trust_address: trust_address.clone(),
            ldt_denom: msg.ldt_denom.clone(),
            redemption_marker_admin: validated_redemption_marker_admin.clone(),
        },
    )?;

    // Store the contract version info for potential future migrations
    let contract_info = ContractVersion {
        contract: CONTRACT_NAME.to_string(),
        version: CONTRACT_VERSION.to_string(),
    };
    CONTRACT_INFO.save(deps.storage, &contract_info)?;

    // Create the loan dicer token marker with the contract as the admin
    let ldt_marker_messages = create_marker_messages(
        env.contract.address.to_string(),
        Coin {
            denom: msg.ldt_denom.clone(),
            amount: "0".to_string(), // Initial supply of 0, we will mint later during contribution finalization
        },
        vec![AccessGrant {
            address: env.contract.address.to_string(),
            permissions: vec![
                Access::Admin as i32,
                Access::Mint as i32,
                Access::Burn as i32,
                Access::Withdraw as i32,
            ],
        }],
        MarkerType::Coin,
    );

    Ok(Response::new()
        .add_messages(ldt_marker_messages)
        .add_attribute("method", "instantiate_contract"))
}
