use crate::error::ContractError;
use crate::msg::InstantiateContractMsg;
use crate::state::{
    validate_contract_app_metadata, CONFIGURATION, CONTRACT_INFO, CONTRACT_NAME, CONTRACT_VERSION,
};
use crate::util::{create_marker_messages, require_denom_not_address};
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

    validate_contract_app_metadata(&msg.app_metadata)?;

    // An address-shaped LDT denom would resolve a different marker on every denom-keyed lookup,
    // making mint and burn permanently unreachable. Reject it before the marker is created.
    require_denom_not_address(deps.as_ref(), &msg.ldt_denom)?;

    // Store the contract configuration
    CONFIGURATION.save(
        deps.storage,
        &crate::state::Configuration {
            otc_address: validated_otc_address.clone(),
            ldt_denom: msg.ldt_denom.clone(),
            redemption_marker_admin: validated_redemption_marker_admin.clone(),
            app_metadata: msg.app_metadata,
        },
    )?;

    // Store the contract version info for potential future migrations
    let contract_info = ContractVersion {
        contract: CONTRACT_NAME.to_string(),
        version: CONTRACT_VERSION.to_string(),
    };
    CONTRACT_INFO.save(deps.storage, &contract_info)?;

    // Create the LDT marker with the contract as admin and governance recovery enabled.
    // Initial supply is zero; Provenance 1.30+ no longer treats zero or 100% supply as admin.
    let ldt_marker_messages = create_marker_messages(
        env.contract.address.to_string(),
        Coin {
            denom: msg.ldt_denom.clone(),
            amount: "0".to_string(),
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
        true,
    );

    Ok(Response::new()
        .add_messages(ldt_marker_messages)
        .add_attribute("method", "instantiate_contract"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ContractAppMetadata, CONFIGURATION};
    use crate::testing::typed_msgs;
    use cosmwasm_std::testing::{mock_env, MockApi};
    use provwasm_mocks::mock_provenance_dependencies;
    use provwasm_std::types::provenance::marker::v1::{Access, MsgAddMarkerRequest};

    fn base_msg(api: MockApi) -> InstantiateContractMsg {
        InstantiateContractMsg {
            otc_address: api.addr_make("otc").to_string(),
            ldt_denom: "ldt.token.test".to_string(),
            redemption_marker_admin: api.addr_make("redemption_marker_admin").to_string(),
            app_metadata: None,
        }
    }

    #[test]
    fn stores_app_metadata() {
        let mut deps = mock_provenance_dependencies();
        let meta = ContractAppMetadata {
            kind: Some("flow_a".to_string()),
            data: Some(r#"{"x":1}"#.to_string()),
        };

        let mut msg = base_msg(deps.api.clone());
        msg.app_metadata = Some(meta.clone());

        instantiate_contract(deps.as_mut(), mock_env(), msg).unwrap();

        let cfg = CONFIGURATION.load(&deps.storage).unwrap();
        assert_eq!(cfg.app_metadata, Some(meta));
    }

    #[test]
    fn rejects_address_shaped_ldt_denom() {
        let mut deps = mock_provenance_dependencies();
        let mut msg = base_msg(deps.api.clone());
        let address_denom = deps.api.addr_make("address-shaped-denom").to_string();
        msg.ldt_denom = address_denom.clone();

        let err = instantiate_contract(deps.as_mut(), mock_env(), msg).unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_denom
        ));
        assert!(CONFIGURATION.may_load(&deps.storage).unwrap().is_none());
    }

    #[test]
    fn rejects_kind_too_long() {
        let mut deps = mock_provenance_dependencies();
        let mut msg = base_msg(deps.api.clone());
        msg.app_metadata = Some(ContractAppMetadata {
            kind: Some("x".repeat(65)),
            data: None,
        });

        let err = instantiate_contract(deps.as_mut(), mock_env(), msg).unwrap_err();
        assert!(matches!(
            err,
            ContractError::ContractAppMetadataTooLong { field: "kind" }
        ));
    }

    #[test]
    fn rejects_data_too_long() {
        let mut deps = mock_provenance_dependencies();
        let mut msg = base_msg(deps.api.clone());
        msg.app_metadata = Some(ContractAppMetadata {
            kind: None,
            data: Some("y".repeat(1025)),
        });

        let err = instantiate_contract(deps.as_mut(), mock_env(), msg).unwrap_err();
        assert!(matches!(
            err,
            ContractError::ContractAppMetadataTooLong { field: "data" }
        ));
    }

    #[test]
    fn creates_ldt_marker_with_zero_supply_and_governance_control() {
        let mut deps = mock_provenance_dependencies();
        let env = mock_env();
        let contract = env.contract.address.to_string();
        let msg = base_msg(deps.api.clone());

        let res = instantiate_contract(deps.as_mut(), env, msg).unwrap();

        let created: Vec<MsgAddMarkerRequest> = typed_msgs(&res, MsgAddMarkerRequest::TYPE_URL);
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some(("ldt.token.test", "0"))
        );
        assert!(created[0].allow_governance_control);
        assert_eq!(created[0].access_list.len(), 1);
        assert_eq!(created[0].access_list[0].address, contract);
        assert_eq!(
            created[0].access_list[0].permissions,
            vec![
                Access::Admin as i32,
                Access::Mint as i32,
                Access::Burn as i32,
                Access::Withdraw as i32,
            ]
        );
    }
}
