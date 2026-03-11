use std::collections::HashSet;

use crate::error::ContractError;
use cosmwasm_std::{Addr, CosmosMsg, Deps};
use provwasm_std::types::cosmos::base::query::v1beta1::PageRequest;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::{
    marker::v1::{
        AccessGrant, MarkerAccount, MarkerQuerier, MarkerStatus, MarkerType, MsgActivateRequest,
        MsgAddMarkerRequest, MsgFinalizeRequest,
    },
    metadata::v1::MetadataQuerier,
};
use uuid::Uuid;

pub fn get_all_owned_scopes(
    deps: Deps,
    marker_denom: String,
) -> Result<Vec<String>, ContractError> {
    let metadata_querier = MetadataQuerier::new(&deps.querier);

    let mut key: Vec<u8> = vec![];
    let mut all_results: Vec<String> = vec![];
    let mut has_next = true;

    let marker = get_marker(deps, marker_denom.clone())?;
    let marker_address = marker
        .base_account
        .ok_or_else(|| {
            ContractError::MarkerQueryError("Error getting base account for marker".to_string())
        })?
        .address;

    while has_next {
        let page_request = PageRequest {
            key: key.clone(),
            offset: 0,
            limit: 50,
            count_total: false,
            reverse: false,
        };

        let response = metadata_querier
            .value_ownership(marker_address.clone(), false, Some(page_request))
            .map_err(|e| {
                ContractError::ScopeQueryError(format!("value ownership query failed: {}", e))
            })?;

        // Collect the scope UUIDs from the current page and lowercase them to ensure consistency
        response.scope_uuids.iter().for_each(|scope_uuid| {
            all_results.push(scope_uuid.to_string().to_lowercase());
        });

        if let Some(next_key) = response.pagination.and_then(|p| p.next_key) {
            if next_key.is_empty() {
                has_next = false;
            } else {
                key = next_key;
            }
        } else {
            has_next = false;
        }
    }

    Ok(all_results)
}

// id can be a denom or a marker address
pub fn get_marker(deps: Deps, id: String) -> Result<MarkerAccount, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.marker(id)?;
    if let Some(marker) = response.marker {
        return if let Ok(account) = MarkerAccount::try_from(marker) {
            Ok(account)
        } else {
            Err(ContractError::MarkerQueryError(
                "failed to parse marker account".to_string(),
            ))
        };
    } else {
        Err(ContractError::MarkerQueryError(
            "no marker found for id".to_string(),
        ))
    }
}

pub fn get_amount_holding(
    deps: Deps,
    marker_denom: String,
    contract_address: Addr,
) -> Result<String, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.holding(marker_denom, None)?;
    let balance = response
        .balances
        .iter()
        .find(|item| item.address == contract_address.to_string());
    return balance
        .ok_or_else(|| ContractError::MarkerQueryError("no balance found for marker".to_string()))
        .map(|coin| {
            coin.coins
                .first()
                .map(|c| c.amount.to_string())
                .unwrap_or_else(|| "0".to_string())
        });
}

pub fn create_marker_messages(
    admin_address: String,
    coin: Coin,
    access_grants: Vec<AccessGrant>,
    marker_type: MarkerType,
) -> Vec<CosmosMsg> {
    let mut messages: Vec<CosmosMsg> = vec![];

    messages.push(CosmosMsg::from(MsgAddMarkerRequest {
        amount: Some(coin.clone()),
        manager: admin_address.clone(),
        from_address: admin_address.to_string(),
        status: MarkerStatus::Proposed as i32,
        marker_type: marker_type as i32,
        access_list: access_grants,
        supply_fixed: false,
        allow_governance_control: false,
        allow_forced_transfer: false,
        required_attributes: vec![],
        usd_cents: 0,
        volume: 0,
        usd_mills: 0,
    }));

    messages.push(CosmosMsg::from(MsgFinalizeRequest {
        denom: coin.denom.clone(),
        administrator: admin_address.clone(),
    }));

    messages.push(CosmosMsg::from(MsgActivateRequest {
        denom: coin.denom.clone(),
        administrator: admin_address.clone(),
    }));
    messages
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

pub fn validate_and_normalize_scope_uuids(
    scope_uuids: &Vec<String>,
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
    marker_denom: String,
) -> Result<AddressBalance, ContractError> {
    let marker_querier = MarkerQuerier::new(&deps.querier);
    let response = marker_querier.holding(marker_denom, None)?;

    // The number of holders should never be 0 because _someone_ must hold the marker coins. We want to enforce single ownership,
    // so if the count is not 1, we return an error.
    if response.balances.len() != 1 {
        return Err(ContractError::MultipleMarkerHolders);
    }

    // We know there is only one balance, so take the first and only balance
    let balance = response
        .balances
        .iter()
        .next()
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

    Ok(AddressBalance {
        address: balance.address.clone(),
        amount: coin.amount.to_string(),
    })
}

pub struct AddressBalance {
    pub address: String,
    pub amount: String,
}

pub struct Nft {
    pub denom: String,
    pub value_owner: String,
}
