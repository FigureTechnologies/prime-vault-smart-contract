use crate::{
    error::ContractError,
    execute::assert_no_funds,
    pending::register_pending_denom,
    state::{
        ensure_holder_funded_pi_burn, PendingDenomOwner, PendingRedemption, CONFIGURATION,
        MAX_REDEMPTION_REF_LEN, PENDING_REDEMPTIONS, REDEMPTION_COUNTER, REDEMPTION_REFERENCES,
    },
    util::{create_marker_messages, require_denom_not_address},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::provenance::marker::v1::{Access, AccessGrant, MarkerType};
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgWithdrawRequest,
};

pub fn execute_initiate_redemption(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    pool_denom: String,
    num_coins: u64,
    redeemer_address: String,
    redemption_ref: Option<String>,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Make sure num_coins is greater than 0 since we don't want to create a redemption marker with 0 coins
    if num_coins == 0 {
        return Err(ContractError::InvalidNumberOfCoins);
    }

    // Validate the redeemer address before proceeding; this will be stored with the pending redemption
    let redeemer_addr = deps.api.addr_validate(&redeemer_address)?;

    // Read the contract configuration from state
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Make sure the sender is authorized as the OTC
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // ConfirmRedemption takes the burn as attached funds. A restricted PI marker rejects that
    // send, so refuse the redemption before a pool marker and pending record are created.
    ensure_holder_funded_pi_burn(configuration.marker_type)?;

    // Marker queries try AccAddressFromBech32 before denom, so an address-shaped pool denom
    // would resolve a different marker and strand this one along with every scope pooled into
    // it. Reject it here, before the marker is created.
    require_denom_not_address(deps.as_ref(), &pool_denom)?;

    if let Some(ref redemption_ref) = redemption_ref {
        // This is not used by the contract, but it can be helpful for off-chain indexing and tracking of redemptions.
        if redemption_ref.len() > MAX_REDEMPTION_REF_LEN {
            return Err(ContractError::ReferenceTooLong);
        }

        // An empty reference would claim the empty key and block every later record from using
        // one, and lookups by empty string would return an unrelated record.
        if redemption_ref.trim().is_empty() {
            return Err(ContractError::ReferenceEmpty);
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

    // Store the redemption
    let redemption = PendingRedemption {
        burn_ldt_amount: None,
        pool_denom: pool_denom.clone(),
        pooling_complete: false,
        redemption_ref: redemption_ref.clone(),
        redeemer_addr,
    };

    let redemption_id = REDEMPTION_COUNTER.may_load(deps.storage)?.unwrap_or(0);

    // Register the pending denom so that it cannot be used in any other contract transactions
    register_pending_denom(
        deps.storage,
        pool_denom.clone(),
        PendingDenomOwner::Redemption { id: redemption_id },
    )?;
    PENDING_REDEMPTIONS.save(deps.storage, redemption_id, &redemption)?;

    // If a reference is provided, store the mapping to the redemption ID
    if let Some(ref redemption_ref) = redemption_ref {
        REDEMPTION_REFERENCES.save(deps.storage, redemption_ref.clone(), &redemption_id)?;
    }

    // Increment the redemption counter
    REDEMPTION_COUNTER.save(deps.storage, &(redemption_id + 1))?;

    // Create the redemption marker with the specified number of coins and grant access to
    // the contract When the redemption is confirmed, the contract will remove its access and
    // add access for the redemption marker admin. Governance control stays enabled so the
    // pooled scopes remain recoverable if the redemption marker admin key is ever lost.
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
        true,
        false,
        vec![],
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
        .add_attribute("redemption_id", redemption_id.to_string())
        .add_attribute("method", "initiate_redemption"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        PendingDenomOwner, MAX_REDEMPTION_REF_LEN, PENDING_DENOMS, PENDING_REDEMPTIONS,
        REDEMPTION_COUNTER, REDEMPTION_REFERENCES,
    };
    use crate::testing::{typed_msgs, TestCtx, POOL_DENOM};
    use provwasm_std::types::provenance::marker::v1::{MsgAddMarkerRequest, MsgWithdrawRequest};

    #[test]
    fn otc_creates_pending_redemption_without_burn_amount() {
        let mut ctx = TestCtx::new();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let res = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            2,
            redeemer_addr,
            Some("redemption-ref".to_string()),
        )
        .unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "redemption_id")
                .map(|a| a.value.as_str()),
            Some("0")
        );

        let stored = PENDING_REDEMPTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.redeemer_addr, ctx.redeemer);
        assert!(stored.burn_ldt_amount.is_none());
        assert!(!stored.pooling_complete);
        assert!(REDEMPTION_REFERENCES
            .may_load(&ctx.deps.storage, "redemption-ref".to_string())
            .unwrap()
            .is_some());
        assert_eq!(
            PENDING_DENOMS
                .load(&ctx.deps.storage, POOL_DENOM.to_string())
                .unwrap(),
            PendingDenomOwner::Redemption { id: 0 }
        );

        let created: Vec<MsgAddMarkerRequest> = typed_msgs(&res, MsgAddMarkerRequest::TYPE_URL);
        assert_eq!(
            created[0].amount.as_ref().map(|c| c.amount.as_str()),
            Some("2")
        );
        assert!(created[0].allow_governance_control);
        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws[0].to_address, ctx.contract_addr());
    }

    #[test]
    fn rejects_restricted_pi_marker_before_creating_a_redemption() {
        let mut ctx = TestCtx::new();
        ctx.set_restricted_pi_marker();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            2,
            redeemer_addr,
            Some("redemption-ref".to_string()),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::RestrictedMarkerHolderBurnNotSupported
        ));
        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(REDEMPTION_REFERENCES
            .may_load(&ctx.deps.storage, "redemption-ref".to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            stranger_info,
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_address_shaped_pool_denom_before_creating_the_marker() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let address_denom = ctx.deps.api.addr_make("address-shaped-denom").to_string();

        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            address_denom.clone(),
            2,
            redeemer_addr,
            None,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_denom
        ));
        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_zero_coins() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            0,
            redeemer_addr,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidNumberOfCoins));
    }

    #[test]
    fn rejects_duplicate_reference() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info();
        let env = ctx.env.clone();
        let redeemer_addr = ctx.redeemer.to_string();
        execute_initiate_redemption(
            ctx.deps.as_mut(),
            info.clone(),
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            Some("redemption-ref".to_string()),
        )
        .unwrap();

        let env = ctx.env.clone();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            info,
            env,
            "other.pool".to_string(),
            1,
            redeemer_addr,
            Some("redemption-ref".to_string()),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ReferenceAlreadyExists { reference } if reference == "redemption-ref"
        ));
    }

    #[test]
    fn rejects_duplicate_pool_denom() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info();
        let env = ctx.env.clone();
        let redeemer_addr = ctx.redeemer.to_string();
        execute_initiate_redemption(
            ctx.deps.as_mut(),
            info.clone(),
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            None,
        )
        .unwrap();

        let env = ctx.env.clone();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            info,
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::PendingDenomAlreadyInUse { denom } if denom == POOL_DENOM
        ));
    }

    #[test]
    fn rejects_reference_too_long() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            Some("x".repeat(MAX_REDEMPTION_REF_LEN + 1)),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::ReferenceTooLong));
    }

    #[test]
    fn rejects_empty_and_whitespace_only_references() {
        for reference in ["", "   ", "\t\n"] {
            let mut ctx = TestCtx::new();
            let env = ctx.env.clone();
            let otc_info = ctx.otc_info();
            let redeemer_addr = ctx.redeemer.to_string();
            let err = execute_initiate_redemption(
                ctx.deps.as_mut(),
                otc_info,
                env,
                POOL_DENOM.to_string(),
                1,
                redeemer_addr,
                Some(reference.to_string()),
            )
            .unwrap_err();

            assert!(matches!(err, ContractError::ReferenceEmpty));
            assert!(PENDING_REDEMPTIONS
                .may_load(&ctx.deps.storage, 0)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let redeemer_addr = ctx.redeemer.to_string();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            info,
            env,
            POOL_DENOM.to_string(),
            1,
            redeemer_addr,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_invalid_redeemer_address_without_writing_state() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_initiate_redemption(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            1,
            "not-a-bech32-address".to_string(),
            Some("redemption-ref".to_string()),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Std(_)));
        assert!(PENDING_REDEMPTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(REDEMPTION_COUNTER
            .may_load(&ctx.deps.storage)
            .unwrap()
            .is_none());
        assert!(REDEMPTION_REFERENCES
            .may_load(&ctx.deps.storage, "redemption-ref".to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
    }
}
