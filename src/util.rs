use std::collections::HashSet;

use crate::error::ContractError;
use cosmwasm_std::{Addr, CosmosMsg, Deps};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::{
    marker::v1::{
        Access, AccessGrant, MarkerAccount, MarkerQuerier, MarkerStatus, MarkerType,
        MsgActivateRequest, MsgAddAccessRequest, MsgAddMarkerRequest, MsgFinalizeRequest,
    },
    metadata::v1::MetadataQuerier,
};
use uuid::Uuid;

/// Permissions the contract must hold on a marker it takes into custody: exclusive Admin,
/// plus Withdraw/Deposit/Transfer for the rest of the asset lifecycle.
pub const CONTRACT_CUSTODY_PERMISSIONS: [i32; 4] = [
    Access::Admin as i32,
    Access::Withdraw as i32,
    Access::Deposit as i32,
    Access::Transfer as i32,
];

// id can be a denom or a marker address
pub fn get_marker(deps: Deps, id: String) -> Result<MarkerAccount, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.marker(id)?;
    if let Some(marker) = response.marker {
        if let Ok(account) = MarkerAccount::try_from(marker) {
            Ok(account)
        } else {
            Err(ContractError::MarkerQueryError(
                "failed to parse marker account".to_string(),
            ))
        }
    } else {
        Err(ContractError::MarkerQueryError(
            "no marker found for id".to_string(),
        ))
    }
}

pub fn get_amount_holding(
    deps: Deps,
    marker_id: String,
    contract_address: Addr,
) -> Result<AddressBalance, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.holding(marker_id, None)?;
    let balance = response
        .balances
        .iter()
        .find(|item| item.address == contract_address.to_string())
        .ok_or_else(|| {
            ContractError::MarkerQueryError("no balance found for marker".to_string())
        })?;

    if balance.coins.len() != 1 {
        return Err(ContractError::MultipleMarkerBalances);
    }

    let coin = balance
        .coins
        .first()
        .ok_or_else(|| ContractError::BalanceQueryError)?;

    if coin.denom.is_empty() {
        return Err(ContractError::MarkerQueryError(
            "holding coin has empty denom".to_string(),
        ));
    }

    Ok(AddressBalance {
        address: balance.address.clone(),
        amount: coin.amount.to_string(),
        denom: coin.denom.clone(),
    })
}

pub fn create_marker_messages(
    admin_address: String,
    coin: Coin,
    access_grants: Vec<AccessGrant>,
    marker_type: MarkerType,
    allow_governance_control: bool,
) -> Vec<CosmosMsg> {
    vec![
        CosmosMsg::from(MsgAddMarkerRequest {
            amount: Some(coin.clone()),
            manager: admin_address.clone(),
            from_address: admin_address.to_string(),
            status: MarkerStatus::Proposed as i32,
            marker_type: marker_type as i32,
            access_list: access_grants,
            supply_fixed: false,
            allow_governance_control,
            allow_forced_transfer: false,
            required_attributes: vec![],
            usd_cents: 0,
            volume: 0,
            usd_mills: 0,
        }),
        CosmosMsg::from(MsgFinalizeRequest {
            denom: coin.denom.clone(),
            administrator: admin_address.clone(),
        }),
        CosmosMsg::from(MsgActivateRequest {
            denom: coin.denom.clone(),
            administrator: admin_address.clone(),
        }),
    ]
}

pub fn get_nft(deps: Deps, scope_address_str: &str) -> Result<Nft, ContractError> {
    let metadata_querier = MetadataQuerier::new(&deps.querier);
    let response = metadata_querier
        .scope(
            scope_address_str.to_owned(),
            String::from(""),
            String::from(""),
            false,
            false,
            false,
            false,
        )
        .map_err(|_| ContractError::ScopeQueryError("scope query failed".to_string()))?;

    let scope_wrapper = response
        .scope
        .ok_or_else(|| ContractError::ScopeQueryError("no scope in response".to_string()))?;
    let scope = scope_wrapper
        .scope
        .ok_or_else(|| ContractError::ScopeQueryError("no scope in response".to_string()))?;
    let scope_id_info = scope_wrapper
        .scope_id_info
        .ok_or_else(|| ContractError::ScopeQueryError("no scope_id_info in scope".to_string()))?;
    let scope_addr = scope_id_info.scope_addr;

    Ok(Nft {
        denom: format!("nft/{}", scope_addr),
        value_owner: scope.value_owner_address,
    })
}

/// Normalizes a single scope UUID for lookups (lowercase) and validates format.
pub fn validate_and_normalize_scope_uuid(scope_uuid: &str) -> Result<String, ContractError> {
    if Uuid::parse_str(scope_uuid).is_err() {
        return Err(ContractError::InvalidScopeUuidFormat(
            scope_uuid.to_string(),
        ));
    }
    Ok(scope_uuid.to_lowercase())
}

pub fn validate_and_normalize_scope_uuids(
    scope_uuids: &[String],
) -> Result<Vec<String>, ContractError> {
    // Do not allow an empty list
    if scope_uuids.is_empty() {
        return Err(ContractError::NoScopesProvided);
    }

    // Ensure all uuid strings parse correctly as UUIDs.
    if let Some(invalid) = scope_uuids
        .iter()
        .find(|uuid| Uuid::parse_str(uuid).is_err())
    {
        return Err(ContractError::InvalidScopeUuidFormat(invalid.clone()));
    }

    // Lower case all UUIDs to ensure case-insensitive uniqueness and consistency
    let normalized_uuids: Vec<String> = scope_uuids.iter().map(|s| s.to_lowercase()).collect();

    // Ensure the scope IDs are unique within this contribution
    let unique_scope_uuids: HashSet<String> = normalized_uuids.iter().cloned().collect();
    if unique_scope_uuids.len() != normalized_uuids.len() {
        return Err(ContractError::DuplicateScopeUuids);
    }
    Ok(normalized_uuids)
}

pub fn get_single_denom_holder(
    deps: Deps,
    marker_id: String,
) -> Result<AddressBalance, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.holding(marker_id, None)?;

    // The number of holders should never be 0 because _someone_ must hold the marker coins. We want to enforce single ownership,
    // so if the count is not 1, we return an error.
    if response.balances.len() != 1 {
        return Err(ContractError::MultipleMarkerHolders);
    }

    // We know there is only one balance, so take the first and only balance
    let balance = response
        .balances
        .first()
        .ok_or_else(|| ContractError::BalanceQueryError)?;

    // This is unexpected based on the query, but we want to be safe and ensure there is exactly one coin in the balance.
    if balance.coins.len() != 1 {
        return Err(ContractError::MultipleMarkerBalances);
    }

    // We know there is only one coin, so take the first and only coin
    let coin = balance
        .coins
        .first()
        .ok_or_else(|| ContractError::BalanceQueryError)?;

    if coin.denom.is_empty() {
        return Err(ContractError::MarkerQueryError(
            "holding coin has empty denom".to_string(),
        ));
    }

    Ok(AddressBalance {
        address: balance.address.clone(),
        amount: coin.amount.to_string(),
        // Holding queries resolve address-or-denom; the coin denom is the marker actually checked.
        denom: coin.denom.clone(),
    })
}

/// Marker queries try `AccAddressFromBech32` before denom. A denom that parses as
/// an account address can therefore resolve a different marker than denom-only
/// messages. Refuse those denoms at submit so they are never persisted.
pub fn require_denom_not_address(deps: Deps, denom: &str) -> Result<(), ContractError> {
    if deps.api.addr_validate(denom).is_ok() {
        return Err(ContractError::MarkerDenomIsAddress {
            denom: denom.to_string(),
        });
    }
    Ok(())
}

fn require_resolved_denom(requested: &str, resolved: &str) -> Result<(), ContractError> {
    if requested != resolved {
        return Err(ContractError::MarkerDenomResolutionMismatch {
            requested_denom: requested.to_string(),
            resolved_denom: resolved.to_string(),
        });
    }
    Ok(())
}

/// Query a marker by a trusted denom. Errors if the chain resolved a different
/// marker (address-first lookup hitting another marker's account).
pub fn get_marker_by_denom(deps: Deps, denom: String) -> Result<MarkerAccount, ContractError> {
    let marker = get_marker(deps, denom.clone())?;
    require_resolved_denom(&denom, &marker.denom)?;
    Ok(marker)
}

/// Holding query for a trusted denom. Errors if the resolved coin denom differs.
pub fn get_single_denom_holder_by_denom(
    deps: Deps,
    denom: String,
) -> Result<AddressBalance, ContractError> {
    let holding = get_single_denom_holder(deps, denom.clone())?;
    require_resolved_denom(&denom, &holding.denom)?;
    Ok(holding)
}

/// Contract holding query for a trusted denom. Errors if the resolved coin denom differs.
pub fn get_amount_holding_by_denom(
    deps: Deps,
    denom: String,
    contract_address: Addr,
) -> Result<AddressBalance, ContractError> {
    let holding = get_amount_holding(deps, denom.clone(), contract_address)?;
    require_resolved_denom(&denom, &holding.denom)?;
    Ok(holding)
}

/// Look up the marker and its single coin holder for a stored/trusted denom.
/// Rejects unless both queries resolve exactly `denom`.
pub fn resolve_stored_marker_holding(
    deps: Deps,
    denom: String,
) -> Result<(MarkerAccount, AddressBalance), ContractError> {
    let marker = get_marker_by_denom(deps, denom.clone())?;
    let holding = get_single_denom_holder_by_denom(deps, denom)?;
    Ok((marker, holding))
}

/// Look up the marker and its single coin holder for `marker_id`.
///
/// `marker_id` may be a denom or a marker address. Queries accept either, but
/// marker messages only accept a denom — so callers must use the returned
/// `marker.denom` / `holding.denom`, not the original `marker_id`. Errors if
/// the two lookups disagree on denom, or if that denom parses as a bech32 address.
pub fn resolve_marker_holding(
    deps: Deps,
    marker_id: String,
) -> Result<(MarkerAccount, AddressBalance), ContractError> {
    let marker = get_marker(deps, marker_id.clone())?;
    if marker.denom.is_empty() {
        return Err(ContractError::MarkerQueryError(
            "marker has empty denom".to_string(),
        ));
    }

    let holding = get_single_denom_holder(deps, marker_id)?;
    if holding.denom != marker.denom {
        return Err(ContractError::MarkerDenomMismatch {
            holding_denom: holding.denom,
            marker_denom: marker.denom,
        });
    }

    require_denom_not_address(deps, &marker.denom)?;

    Ok((marker, holding))
}

/// TransferCoin only succeeds for Active Restricted markers. Reject anything else
/// in-contract so submit does not persist escrow state and then fail at the keeper
/// with an opaque error.
pub fn require_active_restricted_marker(marker: &MarkerAccount) -> Result<(), ContractError> {
    if marker.status != MarkerStatus::Active as i32 {
        return Err(ContractError::MarkerNotActive {
            status: marker.status,
        });
    }
    if marker.marker_type != MarkerType::Restricted as i32 {
        return Err(ContractError::MarkerNotRestricted {
            marker_type: marker.marker_type,
        });
    }
    Ok(())
}

/// Returns the grant for `address` if it contains every permission in `required`.
pub fn require_marker_permissions(
    access_control: &[AccessGrant],
    address: &str,
    required: &[i32],
) -> Result<AccessGrant, ContractError> {
    let grant = access_control
        .iter()
        .find(|grant| grant.address == address)
        .cloned()
        .ok_or(ContractError::InsufficientMarkerAccess)?;

    if required
        .iter()
        .any(|perm| !grant.permissions.contains(perm))
    {
        return Err(ContractError::InsufficientMarkerAccess);
    }

    Ok(grant)
}

/// `MsgAddAccessRequest` granting this contract Admin, Withdraw, Deposit, and Transfer.
/// The marker module requires the administrator to already hold Admin; the message reverts
/// on-chain when that grant is missing.
pub fn msg_grant_contract_custody(denom: String, contract_address: String) -> CosmosMsg {
    CosmosMsg::from(MsgAddAccessRequest {
        denom,
        administrator: contract_address.clone(),
        access: vec![AccessGrant {
            address: contract_address,
            permissions: CONTRACT_CUSTODY_PERMISSIONS.to_vec(),
        }],
    })
}

pub struct AddressBalance {
    pub address: String,
    pub amount: String,
    pub denom: String,
}

pub struct Nft {
    pub denom: String,
    pub value_owner: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use provwasm_std::types::provenance::marker::v1::Access;

    fn grant(address: &str, permissions: Vec<i32>) -> AccessGrant {
        AccessGrant {
            address: address.to_string(),
            permissions,
        }
    }

    fn marker(status: MarkerStatus, marker_type: MarkerType) -> MarkerAccount {
        MarkerAccount {
            base_account: None,
            manager: String::new(),
            access_control: vec![],
            status: status as i32,
            denom: "test.marker".to_string(),
            supply: "1".to_string(),
            marker_type: marker_type as i32,
            supply_fixed: false,
            allow_governance_control: false,
            allow_forced_transfer: false,
            required_attributes: vec![],
        }
    }

    #[test]
    fn require_active_restricted_marker_accepts_active_restricted() {
        require_active_restricted_marker(&marker(MarkerStatus::Active, MarkerType::Restricted))
            .unwrap();
    }

    #[test]
    fn require_active_restricted_marker_rejects_non_active() {
        let err = require_active_restricted_marker(&marker(
            MarkerStatus::Proposed,
            MarkerType::Restricted,
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::MarkerNotActive {
                status
            } if status == MarkerStatus::Proposed as i32
        ));
    }

    #[test]
    fn require_active_restricted_marker_rejects_unrestricted() {
        let err = require_active_restricted_marker(&marker(MarkerStatus::Active, MarkerType::Coin))
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::MarkerNotRestricted {
                marker_type
            } if marker_type == MarkerType::Coin as i32
        ));
    }

    #[test]
    fn require_marker_permissions_accepts_full_custody_set() {
        let grant = require_marker_permissions(
            &[grant("contract", CONTRACT_CUSTODY_PERMISSIONS.to_vec())],
            "contract",
            &CONTRACT_CUSTODY_PERMISSIONS,
        )
        .unwrap();
        assert_eq!(grant.address, "contract");
    }

    #[test]
    fn require_marker_permissions_rejects_transfer_only() {
        let err = require_marker_permissions(
            &[grant("contract", vec![Access::Transfer as i32])],
            "contract",
            &CONTRACT_CUSTODY_PERMISSIONS,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
    }

    #[test]
    fn require_marker_permissions_rejects_missing_grant() {
        let err = require_marker_permissions(&[], "contract", &[Access::Admin as i32]).unwrap_err();
        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
    }

    #[test]
    fn require_marker_permissions_rejects_missing_admin() {
        let err = require_marker_permissions(
            &[grant(
                "contract",
                vec![
                    Access::Withdraw as i32,
                    Access::Deposit as i32,
                    Access::Transfer as i32,
                ],
            )],
            "contract",
            &[Access::Admin as i32],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
    }

    #[test]
    fn require_denom_not_address_rejects_account_address() {
        let ctx = crate::testing::TestCtx::new();
        let addr = ctx.deps.api.addr_make("address-shaped-denom").to_string();
        let err = require_denom_not_address(ctx.deps.as_ref(), &addr).unwrap_err();
        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == addr
        ));
    }

    #[test]
    fn require_denom_not_address_accepts_ordinary_denom() {
        let ctx = crate::testing::TestCtx::new();
        require_denom_not_address(ctx.deps.as_ref(), crate::testing::MARKER_DENOM).unwrap();
    }

    #[test]
    fn get_marker_by_denom_rejects_address_that_resolves_another_marker() {
        let mut ctx = crate::testing::TestCtx::new();
        let victim_address = ctx.deps.api.addr_make("victim-marker").to_string();
        ctx.mock_restricted_marker_with_address(
            crate::testing::MARKER_DENOM,
            &victim_address,
            "holder",
            "1",
            vec![],
        );
        ctx.mock_restricted_marker(&victim_address, "holder", "1", vec![]);

        let err = get_marker_by_denom(ctx.deps.as_ref(), victim_address.clone()).unwrap_err();
        assert!(matches!(
            err,
            ContractError::MarkerDenomResolutionMismatch {
                requested_denom,
                resolved_denom,
            } if requested_denom == victim_address && resolved_denom == crate::testing::MARKER_DENOM
        ));
    }

    #[test]
    fn get_single_denom_holder_rejects_multiple_holders() {
        let mut ctx = crate::testing::TestCtx::new();
        ctx.mock_restricted_marker_holders(
            crate::testing::MARKER_DENOM,
            vec![("holder-a", "1"), ("holder-b", "2")],
        );

        let err = match get_single_denom_holder(
            ctx.deps.as_ref(),
            crate::testing::MARKER_DENOM.to_string(),
        ) {
            Err(err) => err,
            Ok(_) => panic!("expected multiple holders to be rejected"),
        };
        assert!(matches!(err, ContractError::MultipleMarkerHolders));
    }

    #[test]
    fn get_single_denom_holder_rejects_zero_holders() {
        let mut ctx = crate::testing::TestCtx::new();
        ctx.mock_restricted_marker_holders(crate::testing::MARKER_DENOM, vec![]);

        let err = match get_single_denom_holder(
            ctx.deps.as_ref(),
            crate::testing::MARKER_DENOM.to_string(),
        ) {
            Err(err) => err,
            Ok(_) => panic!("expected zero holders to be rejected"),
        };
        assert!(matches!(err, ContractError::MultipleMarkerHolders));
    }

    #[test]
    fn resolve_marker_holding_rejects_address_shaped_denom() {
        let mut ctx = crate::testing::TestCtx::new();
        let address_shaped_denom = ctx.deps.api.addr_make("address-shaped-denom").to_string();
        ctx.mock_restricted_marker(&address_shaped_denom, "holder", "1", vec![]);

        let err = match resolve_marker_holding(ctx.deps.as_ref(), address_shaped_denom.clone()) {
            Err(err) => err,
            Ok(_) => panic!("expected address-shaped denom to be rejected"),
        };
        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_shaped_denom
        ));
    }
}
