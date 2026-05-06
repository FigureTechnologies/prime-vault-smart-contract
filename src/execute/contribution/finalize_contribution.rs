use crate::error::ContractError;
use crate::state::{
    CONFIGURATION, CONTRIBUTION_REFERENCES, MAX_CONTRACT_APP_METADATA_KIND_LEN,
    PENDING_CONTRIBUTIONS, SCOPE_CONTRIBUTOR,
};
use crate::util::{get_all_owned_scopes, get_marker, get_single_denom_holder};
use cosmwasm_std::{CosmosMsg, DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    MsgDeleteAccessRequest, MsgMintRequest, MsgTransferRequest, MsgWithdrawRequest,
};
use std::collections::HashSet;

pub fn execute_finalize_contribution(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    contribution_id: u64,
    marker_denom: String,
) -> Result<Response, ContractError> {
    // Load the contribution from storage
    let contribution = PENDING_CONTRIBUTIONS.load(deps.storage, contribution_id)?;

    // Get the balance of the marker holder. Only one holder is allowed and an error will be returned if there are multiple holders.
    let balance = get_single_denom_holder(deps.as_ref(), marker_denom.clone())?;

    // Verify the contributor holds the marker coins
    if info.sender.to_string() != balance.address.to_string() {
        return Err(ContractError::UnauthorizedContributorAction);
    }

    // Get all the scopes owned by the marker
    let marker_scope_uuids = get_all_owned_scopes(deps.as_ref(), marker_denom.clone())?;

    // Compare the stored scope IDs with the marker’s current scope IDs. The OTC authorized a specific set of scopes for contribution,
    // so if there is any discrepancy, we need to return an error.
    let stored_hash_set: HashSet<String> = contribution.scope_uuids.iter().cloned().collect();
    let current_hash_set: HashSet<String> = marker_scope_uuids.iter().cloned().collect();
    if stored_hash_set != current_hash_set {
        return Err(ContractError::InconsistentMarkerState);
    }

    let contributor = info.sender.clone();
    for uuid in &contribution.scope_uuids {
        if SCOPE_CONTRIBUTOR
            .may_load(deps.storage, uuid.clone())?
            .is_some()
        {
            return Err(ContractError::ScopeAlreadyContributed {
                scope_uuid: uuid.clone(),
            });
        }
    }
    for uuid in &contribution.scope_uuids {
        SCOPE_CONTRIBUTOR.save(deps.storage, uuid.clone(), &contributor)?;
    }

    // Clear out the contribution details since this contribution is being finalized. Scope→contributor
    // attribution is stored until the scope is redeemed (see finalize_redemption).
    PENDING_CONTRIBUTIONS.remove(deps.storage, contribution_id);
    if let Some(ref contribution_ref) = contribution.contribution_ref {
        CONTRIBUTION_REFERENCES.remove(deps.storage, contribution_ref.to_string());
    }

    // Read the contract configuration to get the LDT denom
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Mint the number of LDT tokens specified in the contribution, which was authorized by the OTC.
    // If the mint amount is 0, we can skip this step
    let mint_msgs: Vec<CosmosMsg> = if contribution.mint_ldt_amount > 0 {
        vec![
            CosmosMsg::from(MsgMintRequest {
                administrator: env.contract.address.to_string(),
                amount: Option::Some(Coin {
                    denom: configuration.ldt_denom.to_string(),
                    amount: contribution.mint_ldt_amount.to_string(),
                }),
            }),
            // Withdraw the minted LDT tokens to the marker holder (contributor)
            CosmosMsg::from(MsgWithdrawRequest {
                denom: configuration.ldt_denom.to_string(),
                administrator: env.contract.address.to_string(),
                to_address: balance.address.to_string(), // Withdraw to the marker holder (contributor)
                amount: vec![Coin {
                    denom: configuration.ldt_denom.to_string(),
                    amount: contribution.mint_ldt_amount.to_string(),
                }],
            }),
        ]
    } else {
        vec![]
    };

    let msg_transfer_marker_coins = MsgTransferRequest {
        amount: Some(Coin {
            denom: marker_denom.to_string(),
            amount: balance.amount,
        }),
        administrator: env.contract.address.to_string(),
        from_address: info.sender.to_string(),
        to_address: env.contract.address.to_string(),
    };

    // Get the marker account
    let marker_account = get_marker(deps.as_ref(), marker_denom.clone())?;

    // Get all access grants on the marker except for the contract itself
    let marker_admins: Vec<String> = marker_account
        .access_control
        .iter()
        .filter_map(|grant| {
            if grant.address != env.contract.address.to_string() {
                Some(grant.address.clone())
            } else {
                None
            }
        })
        .collect();

    // Create the messages that deletes access grants for other addresses. We don't allow other access
    // because we want to ensure the contract has exclusive control over the marker after contribution finalization.
    let msgs_delete_access = marker_admins
        .iter()
        .map(|admin| MsgDeleteAccessRequest {
            administrator: env.contract.address.to_string(),
            removed_address: admin.to_string(),
            denom: marker_denom.to_string(),
        })
        .collect::<Vec<MsgDeleteAccessRequest>>();

    let scope_count = contribution.scope_uuids.len().to_string();

    let mut response = Response::new()
        .add_message(msg_transfer_marker_coins)
        .add_messages(msgs_delete_access)
        .add_messages(mint_msgs)
        .add_attribute("method", "finalize_contribution")
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("contributor", contributor.as_str())
        .add_attribute("marker_denom", marker_denom)
        .add_attribute("mint_ldt_amount", contribution.mint_ldt_amount.to_string())
        .add_attribute("scope_count", scope_count);

    if let Some(ref meta) = configuration.app_metadata {
        if let Some(ref k) = meta.kind {
            let truncated: String = k.chars().take(MAX_CONTRACT_APP_METADATA_KIND_LEN).collect();
            response = response.add_attribute("metadata_kind", truncated);
        }
    }

    Ok(response)
}
