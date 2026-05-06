use crate::error::ContractError;
use crate::state::{PendingContribution, CONTRIBUTION_REFERENCES, MAX_CONTRIBUTION_REF_LEN};
use crate::state::{CONFIGURATION, CONTRIBUTION_COUNTER, PENDING_CONTRIBUTIONS};
use crate::util::validate_and_normalize_scope_uuids;
use cosmwasm_std::{DepsMut, MessageInfo, Response};
use cosmwasm_std::{Order, StdResult, Storage};
use cw_storage_plus::Bound;

pub fn execute_initialize_contribution(
    deps: DepsMut,
    info: MessageInfo,
    scope_uuids: Vec<String>,
    mint_ldt_amount: u64,
    contribution_ref: Option<String>,
) -> Result<Response, ContractError> {
    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    if let Some(ref contribution_ref) = contribution_ref {
        // This is not used by the contract, but it can be helpful for off-chain indexing and tracking of contributions.
        if contribution_ref.len() > MAX_CONTRIBUTION_REF_LEN {
            return Err(ContractError::ReferenceTooLong);
        }

        // Make sure that the reference string isn't already being used
        CONTRIBUTION_REFERENCES
            .may_load(deps.storage, contribution_ref.clone())?
            .map_or(Ok(()), |_| {
                Err(ContractError::ReferenceAlreadyExists {
                    reference: contribution_ref.clone(),
                })
            })?;
    }

    // Validate all scope UUIDs
    let normalized_scope_uuids = validate_and_normalize_scope_uuids(&scope_uuids)?;

    // Check that none of the scopes are already in pending contributions
    verify_scopes_not_already_pending(deps.storage, &normalized_scope_uuids)?;

    // Store the contribution. The OTC is authorizing the contribution of the scopes for mint_ldt_amount LDT tokens.
    let contribution = PendingContribution {
        mint_ldt_amount,
        scope_uuids: normalized_scope_uuids,
        contribution_ref: contribution_ref.clone(),
    };
    let contribution_id = CONTRIBUTION_COUNTER.may_load(deps.storage)?.unwrap_or(0);
    PENDING_CONTRIBUTIONS.save(deps.storage, contribution_id, &contribution)?;

    // If a reference is provided, store the mapping to the contribution ID
    if let Some(ref contribution_ref) = contribution_ref {
        CONTRIBUTION_REFERENCES.save(deps.storage, contribution_ref.clone(), &contribution_id)?;
    }

    // Increment the contribution counter
    CONTRIBUTION_COUNTER.save(deps.storage, &(contribution_id + 1))?;

    Ok(Response::new()
        // Return the contribution ID to be used in the finalize step
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("method", "initialize_contribution"))
}

fn load_pending_contributions(
    storage: &dyn Storage,
    start_after: Option<u64>,
    limit: usize,
) -> StdResult<Vec<(u64, PendingContribution)>> {
    let start = start_after.map(Bound::exclusive);
    PENDING_CONTRIBUTIONS
        .range(storage, start, None, Order::Ascending)
        .take(limit)
        .collect()
}

fn verify_scopes_not_already_pending(
    storage: &dyn Storage,
    scope_uuids: &Vec<String>,
) -> Result<(), ContractError> {
    let mut start_after = None;
    let page_limit = 100;
    let input_set: std::collections::HashSet<_> = scope_uuids.iter().collect();
    loop {
        let page = load_pending_contributions(storage, start_after.clone(), page_limit)?;
        if page.is_empty() {
            break;
        }
        // Check for duplicate scope UUIDs in the current page of contributions
        if page.iter().any(|(_, contribution)| {
            contribution
                .scope_uuids
                .iter()
                .any(|uuid| input_set.contains(uuid))
        }) {
            return Err(ContractError::ScopeAlreadyInPendingContribution);
        }
        start_after = Some(page.last().unwrap().0.clone()); // Use the last key as the next start_after
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Configuration, CONFIGURATION};
    use cosmwasm_std::Addr;
    use provwasm_mocks::mock_provenance_dependencies;

    pub const OTC_ADDR_STR: &str = "otc";
    pub const REDEMPTION_ADMIN_STR: &str = "redemption_marker_admin";

    fn setup_config(deps: DepsMut) {
        let otc_address = Addr::unchecked(OTC_ADDR_STR);
        let redemption_marker_admin = Addr::unchecked(REDEMPTION_ADMIN_STR);
        let config = Configuration {
            otc_address,
            ldt_denom: "ldt.token.test".to_string(),
            redemption_marker_admin,
            app_metadata: None,
        };
        CONFIGURATION.save(deps.storage, &config).unwrap();
    }

    #[test]
    fn test_successful_initialize_contribution() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked(OTC_ADDR_STR),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        // Use one uppercase and one lowercase UUID to verify that the contract is normalizing them correctly
        let scope_uuid_0 = "2E9E2078-2274-4289-BE2C-6D70D46C23D3".to_string();
        let scope_uuid_1 = "383a8854-9661-4d44-a17a-19e95600cf66".to_string();
        let scope_uuids = vec![scope_uuid_0.clone(), scope_uuid_1.clone()];

        let res = execute_initialize_contribution(
            deps.as_mut(),
            info,
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap();

        // Verify the contribution was stored correctly
        let contribution_id_attr = res
            .attributes
            .iter()
            .find(|attr| attr.key == "contribution_id")
            .unwrap();
        let contribution_id: u64 = contribution_id_attr.value.parse().unwrap();
        let stored_contribution = PENDING_CONTRIBUTIONS
            .load(deps.as_mut().storage, contribution_id)
            .unwrap();
        assert_eq!(stored_contribution.mint_ldt_amount, mint_ldt_amount);
        assert_eq!(
            stored_contribution.scope_uuids,
            vec![scope_uuid_0.to_lowercase(), scope_uuid_1]
        );
    }

    #[test]
    fn test_unauthorized_sender() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked("not_otc"),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        let scope_uuids = vec!["2e9e2078-2274-4289-be2c-6d70d46c23d3"
            .to_string()
            .to_string()];

        let err = execute_initialize_contribution(
            deps.as_mut(),
            info,
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn test_empty_scope_uuids() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked(OTC_ADDR_STR),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        let scope_uuids = vec![];

        let err = execute_initialize_contribution(
            deps.as_mut(),
            info,
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::NoScopesProvided));
    }

    #[test]
    fn test_duplicate_scope_uuids() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked(OTC_ADDR_STR),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        let scope_uuids = vec![
            "2e9e2078-2274-4289-be2c-6d70d46c23d3"
                .to_string()
                .to_string(),
            "2e9e2078-2274-4289-be2c-6d70d46c23d3"
                .to_string()
                .to_string(),
        ];

        let err = execute_initialize_contribution(
            deps.as_mut(),
            info,
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::DuplicateScopeUuids));
    }

    #[test]
    fn test_invalid_scope_uuid() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked(OTC_ADDR_STR),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        let scope_uuids = vec![
            "2e9e2078-2274-4289-be2c-6d70d46c23d3".to_string(),
            "not-a-uuid".to_string(),
        ];

        let err = execute_initialize_contribution(
            deps.as_mut(),
            info,
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap_err();

        assert!(
            matches!(err, ContractError::InvalidScopeUuidFormat(invalid) if invalid == "not-a-uuid")
        );
    }

    #[test]
    fn test_disallow_overlapping_scope_uuids() {
        let mut deps = mock_provenance_dependencies();
        setup_config(deps.as_mut());

        let info = MessageInfo {
            sender: Addr::unchecked(OTC_ADDR_STR),
            funds: vec![],
        };
        let mint_ldt_amount = 100u64;
        // Use one uppercase and one lowercase UUID to verify that the contract is normalizing them correctly
        let scope_uuid_0 = "2E9E2078-2274-4289-BE2C-6D70D46C23D3".to_string();
        let scope_uuid_1 = "383a8854-9661-4d44-a17a-19e95600cf66".to_string();
        let scope_uuids = vec![scope_uuid_0.clone(), scope_uuid_1.clone()];

        execute_initialize_contribution(
            deps.as_mut(),
            info.clone(),
            scope_uuids,
            mint_ldt_amount,
            None,
        )
        .unwrap();

        let err = execute_initialize_contribution(
            deps.as_mut(),
            info,
            vec![scope_uuid_0.clone()],
            mint_ldt_amount,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeAlreadyInPendingContribution
        ))
    }
}
