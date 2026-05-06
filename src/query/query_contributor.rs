use cosmwasm_std::Deps;

use crate::{
    error::ContractError, msg::GetContributorForScopeResponse, state::SCOPE_CONTRIBUTOR,
    util::validate_and_normalize_scope_uuid,
};

pub fn query_contributor_for_scope(
    deps: Deps,
    scope_uuid: String,
) -> Result<GetContributorForScopeResponse, ContractError> {
    let normalized = validate_and_normalize_scope_uuid(&scope_uuid)?;
    let contributor = SCOPE_CONTRIBUTOR.may_load(deps.storage, normalized.clone())?;
    Ok(GetContributorForScopeResponse {
        scope_uuid: normalized,
        contributor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::clear_contributor_attribution_for_scopes;
    use cosmwasm_std::testing::mock_dependencies;
    use cosmwasm_std::Addr;

    #[test]
    fn contributor_for_scope_round_trip() {
        let mut deps = mock_dependencies();
        let contributor = Addr::unchecked("contributor_addr");
        let scope = "2e9e2078-2274-4289-be2c-6d70d46c23d3".to_string();
        SCOPE_CONTRIBUTOR
            .save(deps.as_mut().storage, scope.clone(), &contributor)
            .unwrap();

        let upper = scope.to_uppercase();
        let res = query_contributor_for_scope(deps.as_ref(), upper).unwrap();
        assert_eq!(res.scope_uuid, scope);
        assert_eq!(res.contributor, Some(contributor));
    }

    #[test]
    fn contributor_query_clear_removes_contributor() {
        let mut deps = mock_dependencies();
        let contributor = Addr::unchecked("contributor_addr");
        let scope = "2e9e2078-2274-4289-be2c-6d70d46c23d3".to_string();
        SCOPE_CONTRIBUTOR
            .save(deps.as_mut().storage, scope.clone(), &contributor)
            .unwrap();

        let res = query_contributor_for_scope(deps.as_ref(), scope.clone()).unwrap();
        assert_eq!(res.contributor, Some(contributor));

        clear_contributor_attribution_for_scopes(
            deps.as_mut().storage,
            std::slice::from_ref(&scope),
        )
        .unwrap();

        let after = query_contributor_for_scope(deps.as_ref(), scope).unwrap();
        assert_eq!(after.contributor, None);
    }
}
