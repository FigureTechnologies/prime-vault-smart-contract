use crate::{
    error::ContractError,
    state::{CONFIGURATION, PENDING_REDEMPTIONS, REDEMPTION_REFERENCES},
    util::{get_amount_holding, get_marker},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, Uint128};
use provwasm_std::types::{
    cosmos::bank::v1beta1::MsgSend,
    provenance::marker::v1::{
        Access, AccessGrant, MsgAddAccessRequest, MsgBurnRequest, MsgTransferRequest,
    },
};
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgDeleteAccessRequest,
};

pub fn execute_finalize_redemption(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    redemption_id: u64,
) -> Result<Response, ContractError> {
    // Verify that only one coin object was sent with the transaction
    if info.funds.len() != 1 {
        return Err(ContractError::InvalidFundsSent);
    }

    // Get the sent funds
    let coin = info.funds.first().ok_or(ContractError::InvalidFundsSent)?;

    // Read the contract configuration to get the LDT denom
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Load the redemption from storage
    let redemption = PENDING_REDEMPTIONS.load(deps.storage, redemption_id)?;

    // Make sure the sender is the redeemer specified in the redemption record
    if info.sender != redemption.redeemer_addr {
        return Err(ContractError::UnauthorizedRedeemer);
    }

    // Ensure the sent coin is in the correct denomination
    if coin.denom != configuration.ldt_denom {
        return Err(ContractError::InvalidCoinSent);
    }

    // Make sure the sent amount matches the expected burn amount
    if Uint128::from(redemption.burn_ldt_amount) != coin.amount {
        return Err(ContractError::IncorrectRedemptionAmount);
    }

    if !redemption.pooling_complete {
        return Err(ContractError::RedemptionPoolingIncomplete);
    }

    // The LDT tokens were sent to the contract, so now the contract can return them to the marker, then burn them.
    let ldt_marker = get_marker(deps.as_ref(), configuration.ldt_denom.clone())?;
    let ldt_marker_base_account = ldt_marker.base_account.ok_or_else(|| {
        ContractError::MarkerQueryError("Error getting base account for marker".to_string())
    })?;

    // Send the ldt tokens back to the marker
    let msg_send_to_marker = MsgSend {
        from_address: env.contract.address.to_string(),
        to_address: ldt_marker_base_account.address.to_string(),
        amount: vec![Coin {
            denom: configuration.ldt_denom.to_string(),
            amount: coin.amount.to_string(),
        }],
    };

    // Burn the LDT tokens
    let msg_burn_ldt = MsgBurnRequest {
        administrator: env.contract.address.to_string(),
        amount: Some(Coin {
            denom: configuration.ldt_denom.to_string(),
            amount: coin.amount.to_string(),
        }),
    };

    // Transfer the scopes to the redeemer
    let amount_holding = get_amount_holding(
        deps.as_ref(),
        redemption.pool_denom.clone(),
        env.contract.address.clone(),
    )?;
    let msg_transfer_scopes = MsgTransferRequest {
        amount: Some(Coin {
            denom: redemption.pool_denom.to_string(),
            amount: amount_holding,
        }),
        administrator: env.contract.address.to_string(),
        from_address: env.contract.address.to_string(),
        to_address: info.sender.to_string(),
    };

    // Set the redemption marker admin as the admin of the marker so they can manage the marker going forward. The contract will remove its access in the next message.
    let msg_add_admin_access = MsgAddAccessRequest {
        denom: redemption.pool_denom.to_string(),
        administrator: env.contract.address.to_string(),
        access: vec![AccessGrant {
            address: configuration.redemption_marker_admin.to_string(),
            permissions: vec![
                Access::Admin as i32,
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ],
        }],
    };

    // Remove the contract's admin access from the marker
    let msg_delete_access = MsgDeleteAccessRequest {
        denom: redemption.pool_denom.to_string(),
        administrator: env.contract.address.to_string(),
        removed_address: env.contract.address.to_string(),
    };

    // Clear out the redemption record since it has been finalized
    PENDING_REDEMPTIONS.remove(deps.storage, redemption_id);
    if let Some(ref redemption_ref) = redemption.redemption_ref {
        REDEMPTION_REFERENCES.remove(deps.storage, redemption_ref.to_string());
    }

    Ok(Response::new()
        .add_message(msg_send_to_marker)
        .add_message(msg_burn_ldt)
        .add_message(msg_transfer_scopes)
        .add_message(msg_add_admin_access)
        .add_message(msg_delete_access)
        .add_attribute("method", "finalize_redemption"))
}
