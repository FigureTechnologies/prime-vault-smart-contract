use crate::{
    error::ContractError,
    execute::assert_no_funds,
    pending::register_pending_denom,
    state::{
        PendingDenomOwner, PendingSwap, CONFIGURATION, MAX_SWAP_REF_LEN, PENDING_SWAPS,
        SWAP_COUNTER, SWAP_REFERENCES,
    },
    util::{create_marker_messages, require_denom_not_address},
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::provenance::marker::v1::{Access, AccessGrant, MarkerType};
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgWithdrawRequest,
};

pub fn execute_initiate_swap(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    pool_denom: String,
    num_coins: u64,
    contributor_address: String,
    swap_ref: Option<String>,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Number of coins is the number of coins that will be created on the swap out marker,
    // which needs to be greater than 0
    if num_coins == 0 {
        return Err(ContractError::InvalidNumberOfCoins);
    }

    // Validate the contributor address
    let contributor_addr = deps.api.addr_validate(&contributor_address)?;

    // Load the contract configuration to access relevant settings
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Verify that the sender is the authorized OTC address
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Marker queries try AccAddressFromBech32 before denom, so an address-shaped pool denom
    // would resolve a different marker and strand this one along with every scope pooled into
    // it. Reject it here, before the marker is created.
    require_denom_not_address(deps.as_ref(), &pool_denom)?;

    if let Some(ref swap_ref) = swap_ref {
        if swap_ref.len() > MAX_SWAP_REF_LEN {
            return Err(ContractError::ReferenceTooLong);
        }

        // An empty reference would claim the empty key and block every later record from using
        // one, and lookups by empty string would return an unrelated record.
        if swap_ref.trim().is_empty() {
            return Err(ContractError::ReferenceEmpty);
        }

        // Store the reference
        SWAP_REFERENCES
            .may_load(deps.storage, swap_ref.clone())?
            .map_or(Ok(()), |_| {
                Err(ContractError::ReferenceAlreadyExists {
                    reference: swap_ref.clone(),
                })
            })?;
    }

    // Create the pending swap record
    let swap = PendingSwap {
        pool_denom: pool_denom.clone(),
        pooling_complete: false,
        contributor_addr,
        swap_ref: swap_ref.clone(),
        mint_ldt_amount: 0,
        burn_ldt_amount: 0,
        incoming_marker_denom: None,
        stored_incoming_access_grants: None,
    };

    // Get the next swap ID
    let swap_id = SWAP_COUNTER.may_load(deps.storage)?.unwrap_or(0);

    // Register the pending denom so that it cannot be used in any other contract transactions
    register_pending_denom(
        deps.storage,
        pool_denom.clone(),
        PendingDenomOwner::Swap { id: swap_id },
    )?;

    // Save the pending swap record and reference
    PENDING_SWAPS.save(deps.storage, swap_id, &swap)?;
    if let Some(ref swap_ref) = swap_ref {
        SWAP_REFERENCES.save(deps.storage, swap_ref.clone(), &swap_id)?;
    }

    // Increment the swap counter so the next swap gets a unique ID
    SWAP_COUNTER.save(deps.storage, &(swap_id + 1))?;

    // Create the pool marker for swapping out assets.
    // Governance control stays enabled so the pooled scopes remain recoverable if the
    // redemption marker admin key is ever lost.
    let pool_marker_messages = create_marker_messages(
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
    );

    // Withdraw the coins from the pool marker to the contract to keep
    // the ownership model consistent during pooling (contract owns the pooled assets)
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
        .add_messages(pool_marker_messages)
        .add_message(msg_withdraw)
        .add_attribute("swap_id", swap_id.to_string())
        .add_attribute("method", "initiate_swap"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        PendingDenomOwner, MAX_SWAP_REF_LEN, PENDING_DENOMS, PENDING_SWAPS, SWAP_REFERENCES,
    };
    use crate::testing::{typed_msgs, TestCtx, POOL_DENOM};
    use provwasm_std::types::provenance::marker::v1::{
        MsgActivateRequest, MsgAddMarkerRequest, MsgWithdrawRequest,
    };

    #[test]
    fn otc_creates_pending_swap_and_pool_marker() {
        let mut ctx = TestCtx::new();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        let res = execute_initiate_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            3,
            contributor_addr,
            Some("swap-ref".to_string()),
        )
        .unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "swap_id")
                .map(|a| a.value.as_str()),
            Some("0")
        );

        let swap = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(swap.contributor_addr, ctx.contributor);
        assert_eq!(swap.pool_denom, POOL_DENOM);
        assert!(!swap.pooling_complete);
        assert_eq!(swap.mint_ldt_amount, 0);
        assert_eq!(swap.burn_ldt_amount, 0);
        assert!(swap.incoming_marker_denom.is_none());
        assert_eq!(
            PENDING_DENOMS
                .load(&ctx.deps.storage, POOL_DENOM.to_string())
                .unwrap(),
            PendingDenomOwner::Swap { id: 0 }
        );
        assert!(SWAP_REFERENCES
            .may_load(&ctx.deps.storage, "swap-ref".to_string())
            .unwrap()
            .is_some());

        let created: Vec<MsgAddMarkerRequest> = typed_msgs(&res, MsgAddMarkerRequest::TYPE_URL);
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((POOL_DENOM, "3"))
        );
        assert!(created[0].allow_governance_control);
        let activated: Vec<MsgActivateRequest> = typed_msgs(&res, MsgActivateRequest::TYPE_URL);
        assert_eq!(activated.len(), 1);
        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws[0].to_address, ctx.contract_addr());
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            stranger_info,
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
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
        let contributor_addr = ctx.contributor.to_string();
        let address_denom = ctx.deps.api.addr_make("address-shaped-denom").to_string();

        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            address_denom.clone(),
            3,
            contributor_addr,
            None,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_denom
        ));
        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, address_denom)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_zero_coins() {
        let mut ctx = TestCtx::new();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            0,
            contributor_addr,
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
        let contributor_addr = ctx.contributor.to_string();
        execute_initiate_swap(
            ctx.deps.as_mut(),
            info.clone(),
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
            Some("swap-ref".to_string()),
        )
        .unwrap();

        let env = ctx.env.clone();
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            info,
            env,
            "other.pool".to_string(),
            1,
            contributor_addr,
            Some("swap-ref".to_string()),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::ReferenceAlreadyExists { reference } if reference == "swap-ref"
        ));
    }

    #[test]
    fn rejects_duplicate_pool_denom() {
        let mut ctx = TestCtx::new();
        let info = ctx.otc_info();
        let env = ctx.env.clone();
        let contributor_addr = ctx.contributor.to_string();
        execute_initiate_swap(
            ctx.deps.as_mut(),
            info.clone(),
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
            None,
        )
        .unwrap();

        let env = ctx.env.clone();
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            info,
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
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
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
            Some("x".repeat(MAX_SWAP_REF_LEN + 1)),
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
            let contributor_addr = ctx.contributor.to_string();
            let err = execute_initiate_swap(
                ctx.deps.as_mut(),
                otc_info,
                env,
                POOL_DENOM.to_string(),
                1,
                contributor_addr,
                Some(reference.to_string()),
            )
            .unwrap_err();

            assert!(matches!(err, ContractError::ReferenceEmpty));
            assert!(PENDING_SWAPS
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
        let contributor_addr = ctx.contributor.to_string();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            info,
            env,
            POOL_DENOM.to_string(),
            1,
            contributor_addr,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_invalid_contributor_address_without_writing_state() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_initiate_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            POOL_DENOM.to_string(),
            1,
            "not-a-bech32-address".to_string(),
            Some("swap-ref".to_string()),
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Std(_)));
        assert!(PENDING_SWAPS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(SWAP_COUNTER.may_load(&ctx.deps.storage).unwrap().is_none());
        assert!(SWAP_REFERENCES
            .may_load(&ctx.deps.storage, "swap-ref".to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, POOL_DENOM.to_string())
            .unwrap()
            .is_none());
    }
}
