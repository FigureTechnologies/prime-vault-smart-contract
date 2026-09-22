use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::state::{PendingContribution, CONTRIBUTION_REFERENCES, MAX_CONTRIBUTION_REF_LEN};
use crate::state::{CONFIGURATION, CONTRIBUTION_COUNTER, PENDING_CONTRIBUTIONS};
use cosmwasm_std::{DepsMut, MessageInfo, Response};

pub fn execute_initiate_contribution(
    deps: DepsMut,
    info: MessageInfo,
    contributor_addr: String,
    contribution_ref: Option<String>,
) -> Result<Response, ContractError> {
    assert_no_funds(&info)?;

    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Validate the contributor address before proceeding; this will be stored with the pending contribution
    let contributor_addr = deps.api.addr_validate(&contributor_addr)?;

    if let Some(ref contribution_ref) = contribution_ref {
        // This is not used by the contract, but it can be helpful for off-chain indexing and tracking of contributions.
        if contribution_ref.len() > MAX_CONTRIBUTION_REF_LEN {
            return Err(ContractError::ReferenceTooLong);
        }

        // An empty reference would claim the empty key and block every later record from using
        // one, and lookups by empty string would return an unrelated record.
        if contribution_ref.trim().is_empty() {
            return Err(ContractError::ReferenceEmpty);
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

    // Store the contribution. The OTC is authorizing the creation of a new pending contribution that
    // can be contributed by the specified contributor.
    let contribution = PendingContribution {
        contributor_addr,
        contribution_ref: contribution_ref.clone(),
        marker_denom: None,
        mint_ldt_amount: None,
        stored_access_grants: None,
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
        // Return the contribution ID to be used in the confirm step
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("method", "initiate_contribution"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MAX_CONTRIBUTION_REF_LEN, PENDING_CONTRIBUTIONS};
    use crate::testing::TestCtx;

    #[test]
    fn otc_creates_pending_contribution_without_marker() {
        let mut ctx = TestCtx::new();

        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        let res = execute_initiate_contribution(
            ctx.deps.as_mut(),
            otc_info,
            contributor_addr,
            Some("contrib-ref".to_string()),
        )
        .unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "contribution_id")
                .map(|a| a.value.as_str()),
            Some("0")
        );

        let stored = PENDING_CONTRIBUTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.contributor_addr, ctx.contributor);
        assert_eq!(stored.contribution_ref.as_deref(), Some("contrib-ref"));
        assert!(stored.marker_denom.is_none());
        assert!(stored.stored_access_grants.is_none());
        assert!(CONTRIBUTION_REFERENCES
            .may_load(&ctx.deps.storage, "contrib-ref".to_string())
            .unwrap()
            .is_some());
    }

    #[test]
    fn assigns_sequential_ids() {
        let mut ctx = TestCtx::new();

        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        execute_initiate_contribution(ctx.deps.as_mut(), otc_info, contributor_addr, None).unwrap();
        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        let res =
            execute_initiate_contribution(ctx.deps.as_mut(), otc_info, contributor_addr, None)
                .unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "contribution_id")
                .map(|a| a.value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();

        let stranger_info = ctx.stranger_info();
        let contributor_addr = ctx.contributor.to_string();
        let err =
            execute_initiate_contribution(ctx.deps.as_mut(), stranger_info, contributor_addr, None)
                .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_duplicate_reference() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info();

        let contributor_addr = ctx.contributor.to_string();
        execute_initiate_contribution(
            ctx.deps.as_mut(),
            info.clone(),
            contributor_addr,
            Some("same-ref".to_string()),
        )
        .unwrap();

        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_contribution(
            ctx.deps.as_mut(),
            info,
            contributor_addr,
            Some("same-ref".to_string()),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::ReferenceAlreadyExists { reference } if reference == "same-ref"
        ));
    }

    #[test]
    fn rejects_reference_too_long() {
        let mut ctx = TestCtx::new();

        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_contribution(
            ctx.deps.as_mut(),
            otc_info,
            contributor_addr,
            Some("x".repeat(MAX_CONTRIBUTION_REF_LEN + 1)),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::ReferenceTooLong));
    }

    #[test]
    fn rejects_empty_and_whitespace_only_references() {
        for reference in ["", "   ", "\t\n"] {
            let mut ctx = TestCtx::new();
            let otc_info = ctx.otc_info();
            let contributor_addr = ctx.contributor.to_string();
            let err = execute_initiate_contribution(
                ctx.deps.as_mut(),
                otc_info,
                contributor_addr,
                Some(reference.to_string()),
            )
            .unwrap_err();

            assert!(matches!(err, ContractError::ReferenceEmpty));
            assert!(PENDING_CONTRIBUTIONS
                .may_load(&ctx.deps.storage, 0)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_contribution(ctx.deps.as_mut(), info, contributor_addr, None)
            .unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_invalid_contributor_address_without_writing_state() {
        let mut ctx = TestCtx::new();
        let otc_info = ctx.otc_info();
        let err = execute_initiate_contribution(
            ctx.deps.as_mut(),
            otc_info,
            "not-a-bech32-address".to_string(),
            Some("contrib-ref".to_string()),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Std(_)));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(CONTRIBUTION_COUNTER
            .may_load(&ctx.deps.storage)
            .unwrap()
            .is_none());
        assert!(CONTRIBUTION_REFERENCES
            .may_load(&ctx.deps.storage, "contrib-ref".to_string())
            .unwrap()
            .is_none());
    }
}
