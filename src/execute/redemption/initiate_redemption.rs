use crate::{
    error::ContractError,
    state::{
        PendingRedemption, CONFIGURATION, MAX_REDEMPTION_REF_LEN, PENDING_REDEMPTIONS,
        REDEMPTION_COUNTER, REDEMPTION_REFERENCES,
    },
    util::{create_marker_messages, validate_and_normalize_scope_uuids},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use cosmwasm_std::{Order, StdResult, Storage};
use cw_storage_plus::Bound;
use provwasm_std::types::provenance::marker::v1::{Access, AccessGrant, MarkerType};
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgWithdrawRequest,
};

pub fn execute_initiate_redemption(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    burn_ldt_amount: u64,
    scope_uuids: Vec<String>,
    pool_denom: String,
    num_coins: u64,
    redeemer_address: String,
    redemption_ref: Option<String>,
) -> Result<Response, ContractError> {
    // Make sure num_coins is greater than 0 since we don't want to create a redemption marker with 0 coins
    if num_coins == 0 {
        return Err(ContractError::InvalidNumberOfCoins);
    }

    if burn_ldt_amount == 0 {
        return Err(ContractError::InvalidBurnAmount);
    }

    // Validate the redeemer address before proceeding; this will be stored with the pending redemption
    let redeemer_addr = deps.api.addr_validate(&redeemer_address)?;

    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    if let Some(ref redemption_ref) = redemption_ref {
        // This is not used by the contract, but it can be helpful for off-chain indexing and tracking of redemptions.
        if redemption_ref.len() > MAX_REDEMPTION_REF_LEN {
            return Err(ContractError::ReferenceTooLong);
        }

        // Make sure that the reference string isn't already being used
        REDEMPTION_REFERENCES
            .may_load(deps.storage, redemption_ref.clone())?
            .map_or(Ok(()), |_| {
                Err(ContractError::ReferenceAlreadyExists {
                    reference: redemption_ref.clone(),
                })
            })?;
    }

    // Validate all scope UUIDs
    let normalized_scope_uuids = validate_and_normalize_scope_uuids(&scope_uuids)?;

    // Verify that none of the scopes are already in pending redemptions
    verify_scopes_not_already_pending(deps.storage, &normalized_scope_uuids)?;

    // Store the redemption. The OTC is authorizing the redemption of the scopes for burn_ldt_amount LDT tokens.
    let redemption = PendingRedemption {
        burn_ldt_amount,
        scope_uuids: normalized_scope_uuids,
        pool_denom: pool_denom.clone(),
        pooling_complete: false,
        redemption_ref: redemption_ref.clone(),
        redeemer_addr,
    };

    let redemption_id = REDEMPTION_COUNTER.may_load(deps.storage)?.unwrap_or(0);
    PENDING_REDEMPTIONS.save(deps.storage, redemption_id, &redemption)?;

    // If a reference is provided, store the mapping to the redemption ID
    if let Some(ref redemption_ref) = redemption_ref {
        REDEMPTION_REFERENCES.save(deps.storage, redemption_ref.clone(), &redemption_id)?;
    }

    // Increment the redemption counter
    REDEMPTION_COUNTER.save(deps.storage, &(redemption_id + 1))?;

    // Create the redemption marker with the specified number of coins and grant access to
    // the contract When the redemption is finalized, the contract will remove its access and
    // add access for the redemption marker admin
    let redemption_marker_messages = create_marker_messages(
        env.contract.address.to_string(),
        Coin {
            denom: pool_denom.clone(),
            amount: num_coins.to_string(),
        },
        vec![AccessGrant {
            address: env.contract.address.to_string(),
            permissions: vec![
                Access::Admin as i32,
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ],
        }],
        MarkerType::Restricted,
    );

    // Withdraw the coints from the redemption marker into the contract's address
    let msg_withdraw = MsgWithdrawRequest {
        denom: pool_denom.clone(),
        administrator: env.contract.address.to_string(),
        to_address: env.contract.address.to_string(),
        amount: vec![Coin {
            denom: pool_denom,
            amount: num_coins.to_string(),
        }],
    };

    Ok(Response::new()
        .add_messages(redemption_marker_messages)
        .add_message(msg_withdraw)
        // Return the redemption ID to be used in the finalize step
        .add_attribute("redemption_id", redemption_id.to_string())
        .add_attribute("method", "initiate_redemption"))
}

fn load_pending_redemptions(
    storage: &dyn Storage,
    start_after: Option<u64>,
    limit: usize,
) -> StdResult<Vec<(u64, PendingRedemption)>> {
    let start = start_after.map(Bound::exclusive);
    PENDING_REDEMPTIONS
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
        let page = load_pending_redemptions(storage, start_after.clone(), page_limit)?;
        if page.is_empty() {
            break;
        }
        // Check for duplicate scope UUIDs in the current page of redemptions
        if page.iter().any(|(_, redemption)| {
            redemption
                .scope_uuids
                .iter()
                .any(|uuid| input_set.contains(uuid))
        }) {
            return Err(ContractError::ScopeAlreadyInPendingRedemption);
        }
        start_after = Some(page.last().unwrap().0.clone()); // Use the last key as the next start_after
    }
    Ok(())
}
