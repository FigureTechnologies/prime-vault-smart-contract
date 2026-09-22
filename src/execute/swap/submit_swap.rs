use crate::{
    error::ContractError,
    pending::register_pending_denom,
    state::{PendingDenomOwner, StoredAccessGrant, CONFIGURATION, PENDING_SWAPS},
    util::{
        msg_grant_contract_custody, require_active_restricted_marker, require_marker_permissions,
        resolve_marker_holding,
    },
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, Uint128};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    Access, MsgDeleteAccessRequest, MsgTransferRequest,
};

pub fn execute_submit_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    swap_id: u64,
    incoming_marker_denom: String,
) -> Result<Response, ContractError> {
    // Load the contract configuration and the pending swap record
    let configuration = CONFIGURATION.load(deps.storage)?;
    let mut swap = PENDING_SWAPS.load(deps.storage, swap_id)?;

    // Ensure the sender is the swap contributor
    if info.sender != swap.contributor_addr {
        return Err(ContractError::UnauthorizedSwapContributor);
    }

    // Ensure the swap pooling has been completed before submission
    if !swap.pooling_complete {
        return Err(ContractError::SwapPoolingIncomplete);
    }

    // Ensure the incoming marker denom has not already been set for this swap
    if swap.incoming_marker_denom.is_some() {
        return Err(ContractError::SwapIncomingDenomAlreadySet);
    }

    // Check if we are burning tokens for this swap
    if swap.burn_ldt_amount > 0 {
        // Only one coin should have been sent
        if info.funds.len() != 1 {
            return Err(ContractError::InvalidFundsSent);
        }
        let coin = info.funds.first().ok_or(ContractError::InvalidFundsSent)?;

        // The coin must be the LDT token specified in the contract configuration
        if coin.denom != configuration.ldt_denom {
            return Err(ContractError::InvalidCoinSent);
        }

        // Verify that the amount of the coin matches the burn amount specified in the swap record
        if Uint128::from(swap.burn_ldt_amount) != coin.amount {
            return Err(ContractError::IncorrectTokenAmount);
        }
    } else if !info.funds.is_empty() {
        return Err(ContractError::InvalidFundsSent);
    }

    // Read the incoming marker and balance
    let (incoming_marker_account, balance) =
        resolve_marker_holding(deps.as_ref(), incoming_marker_denom)?;

    // TransferCoin only succeeds for Active Restricted markers. Check here so a
    // Proposed/Cancelled or Coin marker fails with a contract error instead of an
    // opaque keeper rejection after PENDING_SWAPS has already been written.
    require_active_restricted_marker(&incoming_marker_account)?;

    // Never escrow the LDT marker itself. Submit strips every other access grant, and cancel
    // then deletes the contract's own grant, which would destroy the contract's Mint, Burn, and
    // Admin authority over LDT. Compare the resolved denom, not the caller's string, because
    // resolve_marker_holding also accepts a marker address.
    if incoming_marker_account.denom == configuration.ldt_denom {
        return Err(ContractError::CannotEscrowLdtMarker {
            denom: incoming_marker_account.denom.clone(),
        });
    }

    // Requiring the contributor to be the sole holder is also what keeps the removed and incoming
    // scope sets disjoint: pool_swap only accepts source markers the contract solely holds, and a
    // marker cannot be solely held by both. Pooling is closed before submit opens, and a scope has
    // one value owner, so no explicit intersection check is needed.
    if info.sender.to_string() != balance.address {
        return Err(ContractError::UnauthorizedContributorAction);
    }

    // Marker queries resolve address-or-denom; marker messages resolve by denom only.
    // Always dispatch and persist the resolved denom so checks and custody target the same marker.
    let incoming_marker_denom = incoming_marker_account.denom.clone();
    let incoming_marker_address = incoming_marker_account
        .base_account
        .ok_or_else(|| {
            ContractError::MarkerQueryError(
                "Error getting base account for incoming marker".to_string(),
            )
        })?
        .address;

    let contract_address = env.contract.address.to_string();

    // Store the incoming access grants that are not owned by the contract itself
    let stored_incoming_access_grants: Vec<StoredAccessGrant> = incoming_marker_account
        .access_control
        .iter()
        .filter(|grant| grant.address != contract_address)
        .map(|grant| StoredAccessGrant {
            address: grant.address.clone(),
            permissions: grant.permissions.clone(),
        })
        .collect();

    // Admin is required to grant the rest of the custody set and to delete other parties'
    // grants. Transfer-only (or any Admin-less) configuration is rejected here so we never
    // escrow a marker the contract cannot administer or withdraw from at confirm.
    require_marker_permissions(
        &incoming_marker_account.access_control,
        &contract_address,
        &[Access::Admin as i32],
    )?;

    // Register the pending denom so that it cannot be used in any other contract transactions
    register_pending_denom(
        deps.storage,
        incoming_marker_denom.clone(),
        PendingDenomOwner::Swap { id: swap_id },
    )?;

    // Store the incoming marker denom and the access grants in the swap record
    swap.incoming_marker_denom = Some(incoming_marker_denom.clone());
    swap.stored_incoming_access_grants = Some(stored_incoming_access_grants.clone());
    PENDING_SWAPS.save(deps.storage, swap_id, &swap)?;

    // Grant the full custody set before transferring coins so confirm and cancel
    // can MsgWithdrawRequest escrowed coins. MsgAddAccessRequest reverts on-chain if
    // Admin is absent; the query check above makes that failure explicit.
    let grant_msgs = vec![msg_grant_contract_custody(
        incoming_marker_denom.clone(),
        contract_address,
    )];

    // Transfer the incoming marker coins to the marker
    let msg_transfer_incoming_marker = MsgTransferRequest {
        amount: Some(Coin {
            denom: balance.denom,
            amount: balance.amount,
        }),
        administrator: env.contract.address.to_string(),
        from_address: info.sender.to_string(),
        to_address: incoming_marker_address.to_string(),
    };

    // Remove all access except for the contract
    let msgs_delete_access = stored_incoming_access_grants
        .iter()
        .map(|grant| MsgDeleteAccessRequest {
            administrator: env.contract.address.to_string(),
            removed_address: grant.address.clone(),
            denom: incoming_marker_denom.to_string(),
        })
        .collect::<Vec<MsgDeleteAccessRequest>>();

    let response = Response::new()
        .add_messages(grant_msgs)
        .add_message(msg_transfer_incoming_marker)
        .add_messages(msgs_delete_access)
        .add_attribute("method", "submit_swap")
        .add_attribute("swap_id", swap_id.to_string())
        .add_attribute("contributor", swap.contributor_addr.as_str())
        .add_attribute("incoming_marker_denom", incoming_marker_denom)
        .add_attribute("burn_ldt_amount", swap.burn_ldt_amount.to_string());

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingDenomOwner, PendingSwap, PENDING_DENOMS, PENDING_SWAPS};
    use crate::testing::{
        attr, contract_access, typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM, POOL_DENOM,
    };
    use cosmwasm_std::{Coin as StdCoin, Uint128};
    use provwasm_std::types::provenance::marker::v1::{
        Access, AccessGrant, MarkerStatus, MarkerType, MsgAddAccessRequest, MsgDeleteAccessRequest,
        MsgTransferRequest,
    };

    fn seed_swap(ctx: &mut TestCtx, pooling_complete: bool, burn: u64, incoming: Option<String>) {
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingSwap {
                    pool_denom: POOL_DENOM.to_string(),
                    pooling_complete,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: None,
                    mint_ldt_amount: 0,
                    burn_ldt_amount: burn,
                    incoming_marker_denom: incoming,
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
    }

    fn contributor_held_incoming(ctx: &mut TestCtx) {
        contributor_held_incoming_with_permissions(
            ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
        );
    }

    fn contributor_held_incoming_with_permissions(
        ctx: &mut TestCtx,
        contract_permissions: Vec<i32>,
    ) {
        contributor_held_incoming_state(
            ctx,
            contract_permissions,
            MarkerStatus::Active,
            MarkerType::Restricted,
        );
    }

    fn contributor_held_incoming_state(
        ctx: &mut TestCtx,
        contract_permissions: Vec<i32>,
        status: MarkerStatus,
        marker_type: MarkerType,
    ) {
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        ctx.mock_marker(
            MARKER_DENOM,
            &contributor,
            "1",
            vec![
                contract_access(&contract, contract_permissions),
                AccessGrant {
                    address: ctx.stranger.to_string(),
                    permissions: vec![Access::Admin as i32],
                },
            ],
            status,
            marker_type,
        );
    }

    fn assert_full_custody_grant(res: &Response, contract: &str, denom: &str) {
        let grants: Vec<MsgAddAccessRequest> = typed_msgs(res, MsgAddAccessRequest::TYPE_URL);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].denom, denom);
        assert_eq!(grants[0].access[0].address, contract);
        for perm in [
            Access::Admin as i32,
            Access::Withdraw as i32,
            Access::Deposit as i32,
            Access::Transfer as i32,
        ] {
            assert!(grants[0].access[0].permissions.contains(&perm));
        }
    }

    #[test]
    fn contributor_escrows_incoming_marker_after_pooling() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming(&mut ctx);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap();

        assert_eq!(attr(&res, "method"), "submit_swap");
        let stored = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.incoming_marker_denom.as_deref(), Some(MARKER_DENOM));
        assert_eq!(
            PENDING_DENOMS
                .load(&ctx.deps.storage, MARKER_DENOM.to_string())
                .unwrap(),
            PendingDenomOwner::Swap { id: 0 }
        );

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(transfers[0].from_address, ctx.contributor.to_string());
        assert_eq!(
            transfers[0].to_address,
            format!("{MARKER_DENOM}.marker.account")
        );
        assert_eq!(
            transfers[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((MARKER_DENOM, "1"))
        );

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes.iter().all(|msg| msg.denom == MARKER_DENOM));
        assert!(deletes
            .iter()
            .any(|msg| msg.removed_address == ctx.stranger.to_string()));

        assert_full_custody_grant(&res, &ctx.contract_addr(), MARKER_DENOM);
    }

    #[test]
    fn canonicalizes_marker_address_input_to_resolved_denom() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming(&mut ctx);

        let marker_address = format!("{MARKER_DENOM}.marker.account");
        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_swap(ctx.deps.as_mut(), env, contributor_info, 0, marker_address)
            .unwrap();

        let stored = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.incoming_marker_denom.as_deref(), Some(MARKER_DENOM));
        assert_eq!(attr(&res, "incoming_marker_denom"), MARKER_DENOM);

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(
            transfers[0].amount.as_ref().map(|c| c.denom.as_str()),
            Some(MARKER_DENOM)
        );
        assert_eq!(
            transfers[0].to_address,
            format!("{MARKER_DENOM}.marker.account")
        );

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes.iter().all(|msg| msg.denom == MARKER_DENOM));
    }

    #[test]
    fn binds_custody_to_checked_marker_when_decoy_denom_equals_address() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        const REAL_DENOM: &str = "incoming.real";
        const REAL_ADDRESS: &str = "tp1realincomingmarker";
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        let grants = vec![
            contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            ),
            AccessGrant {
                address: ctx.stranger.to_string(),
                permissions: vec![Access::Admin as i32],
            },
        ];
        ctx.mock_restricted_marker_with_address(
            REAL_DENOM,
            REAL_ADDRESS,
            &contributor,
            "9",
            grants.clone(),
        );
        // Decoy marker whose denom is the real marker's bech32 address.
        ctx.mock_restricted_marker(REAL_ADDRESS, &contributor, "9", grants);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            REAL_ADDRESS.to_string(),
        )
        .unwrap();

        let stored = PENDING_SWAPS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.incoming_marker_denom.as_deref(), Some(REAL_DENOM));

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(
            transfers[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((REAL_DENOM, "9"))
        );
        assert_eq!(transfers[0].to_address, REAL_ADDRESS);

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes.iter().all(|msg| msg.denom == REAL_DENOM));
        assert!(deletes
            .iter()
            .any(|msg| msg.removed_address == ctx.stranger.to_string()));
    }

    #[test]
    fn rejects_incoming_marker_whose_denom_parses_as_account_address() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        let address_shaped_denom = ctx.deps.api.addr_make("address-shaped-denom").to_string();
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        let decoy_address = format!("{address_shaped_denom}.marker.account");
        ctx.mock_restricted_marker_with_address(
            &address_shaped_denom,
            &decoy_address,
            &contributor,
            "1",
            vec![contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            )],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(ctx.deps.as_mut(), env, contributor_info, 0, decoy_address)
            .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_shaped_denom
        ));
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_none());
    }

    #[test]
    fn requires_contributor_to_pay_burn_ldt() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 75, None);
        contributor_held_incoming(&mut ctx);

        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&[StdCoin {
            denom: LDT_DENOM.to_string(),
            amount: Uint128::new(75),
        }]);
        let res =
            execute_submit_swap(ctx.deps.as_mut(), env, info, 0, MARKER_DENOM.to_string()).unwrap();

        assert_eq!(attr(&res, "burn_ldt_amount"), "75");
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_some());
    }

    #[test]
    fn rejects_wrong_burn_denom() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 75, None);

        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&[StdCoin {
            denom: "not-ldt".to_string(),
            amount: Uint128::new(75),
        }]);
        let err = execute_submit_swap(ctx.deps.as_mut(), env, info, 0, MARKER_DENOM.to_string())
            .unwrap_err();

        assert!(matches!(err, ContractError::InvalidCoinSent));
    }

    #[test]
    fn rejects_incorrect_burn_amount() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 75, None);

        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&[StdCoin {
            denom: LDT_DENOM.to_string(),
            amount: Uint128::new(10),
        }]);
        let err = execute_submit_swap(ctx.deps.as_mut(), env, info, 0, MARKER_DENOM.to_string())
            .unwrap_err();

        assert!(matches!(err, ContractError::IncorrectTokenAmount));
    }

    #[test]
    fn rejects_missing_ldt_when_burn_required() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 75, None);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_unexpected_funds_when_no_burn() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&[StdCoin {
            denom: LDT_DENOM.to_string(),
            amount: Uint128::new(1),
        }]);
        let err = execute_submit_swap(ctx.deps.as_mut(), env, info, 0, MARKER_DENOM.to_string())
            .unwrap_err();

        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_submit_before_pooling_complete() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, false, 0, None);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::SwapPoolingIncomplete));
    }

    #[test]
    fn rejects_second_submit_after_incoming_is_escrowed() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, Some(MARKER_DENOM.to_string()));

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::SwapIncomingDenomAlreadySet));
    }

    #[test]
    fn rejects_non_contributor() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            stranger_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedSwapContributor));
    }

    #[test]
    fn grants_full_custody_permissions_when_contract_already_has_admin() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming_with_permissions(
            &mut ctx,
            vec![Access::Admin as i32, Access::Withdraw as i32],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap();

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].access[0].address, ctx.contract_addr());
        assert_eq!(
            grants[0].access[0].permissions,
            vec![
                Access::Admin as i32,
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ]
        );
    }

    #[test]
    fn rejects_non_active_marker_before_writing_state() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming_state(
            &mut ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
            MarkerStatus::Proposed,
            MarkerType::Restricted,
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerNotActive {
                status
            } if status == MarkerStatus::Proposed as i32
        ));
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_none());
    }

    #[test]
    fn rejects_unrestricted_marker_before_writing_state() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming_state(
            &mut ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
            MarkerStatus::Active,
            MarkerType::Coin,
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerNotRestricted {
                marker_type
            } if marker_type == MarkerType::Coin as i32
        ));
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_none());
    }

    #[test]
    fn rejects_escrowing_the_ldt_marker() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        // The LDT marker is Coin type today, so require_active_restricted_marker would reject it
        // for an unrelated reason. Mock it Restricted so the denom check is what actually fires.
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(
            LDT_DENOM,
            &contributor,
            "1",
            vec![contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            )],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            LDT_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::CannotEscrowLdtMarker { denom } if denom == LDT_DENOM
        ));
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_none());
    }

    #[test]
    fn rejects_escrowing_the_ldt_marker_supplied_as_its_address() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);

        // Marker queries resolve an address before a denom, so the guard has to compare the
        // resolved denom. Supplying the LDT marker's address must not slip past it.
        let ldt_marker_address = ctx.deps.api.addr_make("ldt-marker").to_string();
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker_with_address(
            LDT_DENOM,
            &ldt_marker_address,
            &contributor,
            "1",
            vec![contract_access(
                &contract,
                vec![Access::Admin as i32, Access::Transfer as i32],
            )],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            ldt_marker_address,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::CannotEscrowLdtMarker { denom } if denom == LDT_DENOM
        ));
    }

    #[test]
    fn rejects_when_contract_has_only_transfer_access() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, true, 0, None);
        contributor_held_incoming_with_permissions(&mut ctx, vec![Access::Transfer as i32]);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_swap(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
        assert!(PENDING_SWAPS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .incoming_marker_denom
            .is_none());
    }
}
