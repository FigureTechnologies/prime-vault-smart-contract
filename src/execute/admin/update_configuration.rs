use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::state::CONFIGURATION;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};

/// Rotates the privileged addresses in configuration. Contract admin only.
///
/// Every privileged route reads `otc_address` at call time, so rotating it hands operational
/// control to the new key immediately, including for records that are already in flight.
///
/// Rotating `redemption_marker_admin` only affects redemptions and swaps confirmed after this
/// call. Pool markers already settled granted custody to the previous admin and are unaffected;
/// recovering those is what governance control on the pool markers is for.
///
/// `ldt_denom` is deliberately not rotatable. It is bound to the marker created at instantiate,
/// so changing it would orphan that marker along with the contract's mint and burn authority.
pub fn execute_update_configuration(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    otc_address: Option<String>,
    redemption_marker_admin: Option<String>,
) -> Result<Response, ContractError> {
    // Ensure the caller has no attached funds
    assert_no_funds(&info)?;

    // Make sure the caller is the contract admin
    let contract_info = deps
        .querier
        .query_wasm_contract_info(env.contract.address.clone())?;
    let admin = contract_info
        .admin
        .ok_or(ContractError::UnauthorizedContractAdmin)?;
    if info.sender != admin {
        return Err(ContractError::UnauthorizedContractAdmin);
    }

    // Ensure that at least one of the configuration fields is being updated
    if otc_address.is_none() && redemption_marker_admin.is_none() {
        return Err(ContractError::EmptyConfigurationUpdate);
    }

    // Load the contract configuration
    let mut configuration = CONFIGURATION.load(deps.storage)?;

    // Keep track of the previous values for auditing purposes
    let previous_otc_address = configuration.otc_address.clone();
    let previous_redemption_marker_admin = configuration.redemption_marker_admin.clone();

    // Update the otc address if a new one is provided
    if let Some(ref otc_address) = otc_address {
        configuration.otc_address = deps.api.addr_validate(otc_address)?;
    }

    // Update the redemption marker admin if a new one is provided
    if let Some(ref redemption_marker_admin) = redemption_marker_admin {
        configuration.redemption_marker_admin = deps.api.addr_validate(redemption_marker_admin)?;
    }

    // Save the new configuration
    CONFIGURATION.save(deps.storage, &configuration)?;

    // Emit both the old and new values so a rotation of contract control is auditable off-chain.
    Ok(Response::new()
        .add_attribute("method", "update_configuration")
        .add_attribute("previous_otc_address", previous_otc_address)
        .add_attribute("otc_address", configuration.otc_address)
        .add_attribute(
            "previous_redemption_marker_admin",
            previous_redemption_marker_admin,
        )
        .add_attribute(
            "redemption_marker_admin",
            configuration.redemption_marker_admin,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute::contribution::initiate_contribution::execute_initiate_contribution;
    use crate::testing::{attr, TestCtx, LDT_DENOM};
    use cosmwasm_std::testing::message_info;

    #[test]
    fn admin_rotates_both_addresses() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());
        let new_otc = ctx.deps.api.addr_make("new-otc").to_string();
        let new_redemption_admin = ctx.deps.api.addr_make("new-redemption-admin").to_string();
        let previous_otc = ctx.otc.to_string();
        let previous_redemption_admin = ctx.redemption_admin.to_string();

        let env = ctx.env.clone();
        let res = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            Some(new_otc.clone()),
            Some(new_redemption_admin.clone()),
        )
        .unwrap();

        assert_eq!(attr(&res, "method"), "update_configuration");
        assert_eq!(attr(&res, "previous_otc_address"), previous_otc);
        assert_eq!(attr(&res, "otc_address"), new_otc);
        assert_eq!(
            attr(&res, "previous_redemption_marker_admin"),
            previous_redemption_admin
        );
        assert_eq!(attr(&res, "redemption_marker_admin"), new_redemption_admin);

        let stored = CONFIGURATION.load(&ctx.deps.storage).unwrap();
        assert_eq!(stored.otc_address.to_string(), new_otc);
        assert_eq!(
            stored.redemption_marker_admin.to_string(),
            new_redemption_admin
        );
        // The LDT denom is not rotatable and must survive untouched.
        assert_eq!(stored.ldt_denom, LDT_DENOM);
    }

    #[test]
    fn omitted_fields_are_left_unchanged() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());
        let new_otc = ctx.deps.api.addr_make("new-otc").to_string();
        let untouched_redemption_admin = ctx.redemption_admin.clone();

        let env = ctx.env.clone();
        execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            Some(new_otc.clone()),
            None,
        )
        .unwrap();

        let stored = CONFIGURATION.load(&ctx.deps.storage).unwrap();
        assert_eq!(stored.otc_address.to_string(), new_otc);
        assert_eq!(stored.redemption_marker_admin, untouched_redemption_admin);

        // And the same in the other direction.
        let new_redemption_admin = ctx.deps.api.addr_make("new-redemption-admin").to_string();
        let env = ctx.env.clone();
        execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            None,
            Some(new_redemption_admin.clone()),
        )
        .unwrap();

        let stored = CONFIGURATION.load(&ctx.deps.storage).unwrap();
        assert_eq!(stored.otc_address.to_string(), new_otc);
        assert_eq!(
            stored.redemption_marker_admin.to_string(),
            new_redemption_admin
        );
    }

    #[test]
    fn rotation_transfers_otc_control_immediately() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());
        let new_otc = ctx.deps.api.addr_make("new-otc");
        let contributor = ctx.contributor.to_string();

        let env = ctx.env.clone();
        execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            Some(new_otc.to_string()),
            None,
        )
        .unwrap();

        // The retired key can no longer drive the contract.
        let old_otc_info = ctx.otc_info();
        let err = execute_initiate_contribution(
            ctx.deps.as_mut(),
            old_otc_info,
            contributor.clone(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));

        // The new one can.
        execute_initiate_contribution(
            ctx.deps.as_mut(),
            message_info(&new_otc, &[]),
            contributor,
            None,
        )
        .unwrap();
    }

    #[test]
    fn rejects_non_admin() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin);
        let new_otc = ctx.deps.api.addr_make("new-otc").to_string();

        // The OTC must not be able to rotate itself or the redemption admin.
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            otc_info,
            Some(new_otc.clone()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedContractAdmin));

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            stranger_info,
            Some(new_otc),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedContractAdmin));

        let stored = CONFIGURATION.load(&ctx.deps.storage).unwrap();
        assert_eq!(stored.otc_address, ctx.otc);
    }

    #[test]
    fn rejects_update_with_no_fields_set() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());

        let env = ctx.env.clone();
        let err = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            None,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::EmptyConfigurationUpdate));
    }

    #[test]
    fn rejects_invalid_address_without_writing_state() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());

        let env = ctx.env.clone();
        let err = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &[]),
            Some("not-a-bech32-address".to_string()),
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::Std(_)));
        assert_eq!(
            CONFIGURATION.load(&ctx.deps.storage).unwrap().otc_address,
            ctx.otc
        );
    }

    #[test]
    fn rejects_when_the_contract_has_no_admin() {
        let mut ctx = TestCtx::new();
        ctx.mock_contract_without_admin();
        let new_otc = ctx.deps.api.addr_make("new-otc").to_string();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();

        let err =
            execute_update_configuration(ctx.deps.as_mut(), env, otc_info, Some(new_otc), None)
                .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedContractAdmin));
        assert_eq!(
            CONFIGURATION.load(&ctx.deps.storage).unwrap().otc_address,
            ctx.otc
        );
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let admin = ctx.deps.api.addr_make("contract-admin");
        ctx.mock_contract_admin(admin.clone());

        let env = ctx.env.clone();
        let err = execute_update_configuration(
            ctx.deps.as_mut(),
            env,
            message_info(&admin, &TestCtx::unexpected_funds()),
            None,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
