use crate::{
    error::ContractError,
    state::{CONFIGURATION, PENDING_REDEMPTIONS},
    util::{get_all_owned_scopes, get_marker, get_nft, get_single_denom_holder},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use itertools::Itertools;
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgWithdrawRequest,
};
use std::collections::HashSet;

pub fn execute_pool_redemption(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    redemption_id: u64,
    num_scopes_to_pool: u32,
) -> Result<Response, ContractError> {
    // Read the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Load the redemption from storage
    let redemption = PENDING_REDEMPTIONS.load(deps.storage, redemption_id)?;
    let owned_scopes = get_all_owned_scopes(deps.as_ref(), redemption.pool_denom.clone())?;

    let redemption_scope_uuids: HashSet<String> = redemption.scope_uuids.iter().cloned().collect();
    let owned_scope_uuids: HashSet<String> = owned_scopes.iter().cloned().collect();

    // Get the IDs in redemption.scope_uuids that are NOT in owned_scopes
    let mut difference: Vec<String> = redemption_scope_uuids
        .difference(&owned_scope_uuids)
        .cloned()
        .collect();

    // Sort the difference to have a consistent order, this is important to make the contract operation deterministic
    difference.sort();

    // Determine if pooling will be complete after this operation
    let pooling_complete = difference.len() <= num_scopes_to_pool as usize;

    // Update the redemption in storage to reflect the state of the pooling
    let mut updated_redemption = redemption.clone();
    updated_redemption.pooling_complete = pooling_complete;
    PENDING_REDEMPTIONS.save(deps.storage, redemption_id, &updated_redemption)?;

    // Get the NFT objects for the scopes to be pooled. We only take
    let scopes_to_pool: Vec<String> = difference
        .into_iter()
        .take(num_scopes_to_pool as usize)
        .collect();
    let nfts = scopes_to_pool
        .iter()
        .map(|scope_uuid| get_nft(deps.as_ref(), scope_uuid))
        .collect::<Result<Vec<_>, ContractError>>()?;

    // Group the NFTs by their value_owner so that we can move them efficiently by owner
    let grouped_nfts = nfts
        .into_iter()
        .into_group_map_by(|nft| nft.value_owner.clone());

    // Get the pool marker base account address so we can deposit the withdrawn NFTs there
    let pool_marker = get_marker(deps.as_ref(), redemption.pool_denom.clone())?;
    let pool_marker_base_account = pool_marker.base_account.ok_or_else(|| {
        ContractError::MarkerQueryError("Error getting base account for marker".to_string())
    })?;

    // Create MsgWithdrawRequest messages for each owner group
    let mut msg_withdraws: Vec<MsgWithdrawRequest> = grouped_nfts
        .into_iter()
        .map(|(owner, nfts)| {
            let marker = get_marker(deps.as_ref(), owner.clone())?;

            // Make sure the contract owns the coins of the marker holding the scopes to be redeemed, in order
            // prevent the contract from redeeming scopes it is authorized to transfer but does not own
            let address_balance = get_single_denom_holder(deps.as_ref(), marker.denom.clone())?;
            if address_balance.address != env.contract.address.to_string() {
                return Err(ContractError::UnauthorizedScopeForRedemption);
            }

            // Create the list of coins to withdraw for this owner group. Each scope is represented as a coin with amount 1.
            let mut coins: Vec<Coin> = nfts
                .iter()
                .map(|nft| Coin {
                    denom: nft.denom.clone(),
                    amount: "1".to_string(),
                })
                .collect();

            // Coins must be sorted by denom
            coins.sort_by(|a, b| a.denom.cmp(&b.denom));

            Ok(MsgWithdrawRequest {
                // Withdraw from the marker owned by the contract
                denom: marker.denom.clone(),
                administrator: env.contract.address.to_string(),
                // Deposit into the pool marker
                to_address: pool_marker_base_account.address.to_string(),
                amount: coins,
            })
        })
        .collect::<Result<Vec<MsgWithdrawRequest>, ContractError>>()?;

    // Sort the withdraw requests by marker denom to ensure a deterministic order of messages
    msg_withdraws.sort_by(|a, b| a.denom.cmp(&b.denom));

    Ok(Response::new()
        .add_messages(msg_withdraws)
        .add_attribute("pooling_complete", pooling_complete.to_string())
        .add_attribute("method", "pool_redemption")
        .add_attribute("redemption_id", redemption_id.to_string()))
}
