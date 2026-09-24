use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::pending::register_pending_denom;
use crate::state::{PendingDenomOwner, StoredAccessGrant, CONFIGURATION, PENDING_CONTRIBUTIONS};
use crate::util::{
    msg_grant_contract_custody, require_active_restricted_marker, require_marker_permissions,
    resolve_marker_holding,
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    Access, MsgDeleteAccessRequest, MsgTransferRequest,
};
pub fn execute_submit_contribution(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    contribution_id: u64,
    marker_denom: String,
) -> Result<Response, ContractError> {
    // Make sure no funds were sent with this request
    assert_no_funds(&info)?;

    // Load the contribution from storage
    let mut contribution = PENDING_CONTRIBUTIONS.load(deps.storage, contribution_id)?;

    // Make sure the sender is the contributor specified in the contribution record
    if info.sender != contribution.contributor_addr {
        return Err(ContractError::UnauthorizedContributorAction);
    }

    // We don't expect the contribution to already have a marker denom set at this point.
    if contribution.marker_denom.is_some() {
        return Err(ContractError::ContributionDenomAlreadySet);
    }

    // Read the marker and balance for the marker_denom
    let (marker_account, balance) = resolve_marker_holding(deps.as_ref(), marker_denom)?;

    // TransferCoin only succeeds for Active Restricted markers. Check here so a
    // Proposed/Cancelled or Coin marker fails with a contract error instead of an
    // opaque keeper rejection after PENDING_CONTRIBUTIONS has already been written.
    require_active_restricted_marker(&marker_account)?;

    // Read the contract configuration to get the LDT denom
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Never escrow the LDT marker itself. Submit strips every other access grant, and cancel
    // then deletes the contract's own grant, which would destroy the contract's Mint, Burn, and
    // Admin authority over LDT. Compare the resolved denom, not the caller's string, because
    // resolve_marker_holding also accepts a marker address.
    if marker_account.denom == configuration.ldt_denom {
        return Err(ContractError::CannotEscrowLdtMarker {
            denom: marker_account.denom.clone(),
        });
    }

    // Verify the contributor holds the marker coins
    if info.sender.to_string() != balance.address {
        return Err(ContractError::UnauthorizedContributorAction);
    }

    // Marker queries resolve address-or-denom; marker messages resolve by denom only.
    // Always dispatch and persist the resolved denom so checks and custody target the same marker.
    let marker_denom = marker_account.denom.clone();
    let marker_address = marker_account
        .base_account
        .ok_or_else(|| {
            ContractError::MarkerQueryError("Error getting base account for marker".to_string())
        })?
        .address;

    let contract_address = env.contract.address.to_string();

    // Get all access grants for addresses other than the contract itself
    let stored_access_grants: Vec<StoredAccessGrant> = marker_account
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
    // escrow a marker the contract cannot administer.
    require_marker_permissions(
        &marker_account.access_control,
        &contract_address,
        &[Access::Admin as i32],
    )?;

    contribution.marker_denom = Some(marker_denom.clone());
    contribution.stored_access_grants = Some(stored_access_grants.clone());

    // Register the pending denom so that it cannot be used in any other contract transactions
    register_pending_denom(
        deps.storage,
        marker_denom.clone(),
        PendingDenomOwner::Contribution {
            id: contribution_id,
        },
    )?;
    PENDING_CONTRIBUTIONS.save(deps.storage, contribution_id, &contribution)?;

    // Transfer the marker coins to the marker account while awaiting finalization of the contribution by the OTC
    let msg_transfer_marker_coins = MsgTransferRequest {
        amount: Some(Coin {
            denom: balance.denom,
            amount: balance.amount,
        }),
        administrator: env.contract.address.to_string(),
        from_address: info.sender.to_string(),
        to_address: marker_address,
    };

    // Grant the full custody set before transferring coins so confirm and cancel
    // can MsgWithdrawRequest escrowed coins. MsgAddAccessRequest reverts
    // on-chain if Admin is absent; the query check above makes that failure explicit.
    let grant_msgs = vec![msg_grant_contract_custody(
        marker_denom.clone(),
        contract_address,
    )];

    // Create the messages that deletes access grants for other addresses. We don't allow other access
    // because we want to ensure the contract has exclusive control over the marker after contribution finalization.
    let msgs_delete_access = stored_access_grants
        .iter()
        .map(|grant| MsgDeleteAccessRequest {
            administrator: env.contract.address.to_string(),
            removed_address: grant.address.clone(),
            denom: marker_denom.to_string(),
        })
        .collect::<Vec<MsgDeleteAccessRequest>>();

    let response = Response::new()
        .add_messages(grant_msgs)
        .add_message(msg_transfer_marker_coins)
        .add_messages(msgs_delete_access)
        .add_attribute("method", "submit_contribution")
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("marker_denom", marker_denom);

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        PendingContribution, PendingDenomOwner, PENDING_CONTRIBUTIONS, PENDING_DENOMS,
    };
    use crate::testing::{attr, contract_access, typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM};
    use provwasm_std::types::provenance::marker::v1::{
        Access, AccessGrant, MarkerStatus, MarkerType, MsgAddAccessRequest, MsgDeleteAccessRequest,
        MsgTransferRequest,
    };

    fn seed_pending(ctx: &mut TestCtx, marker_denom: Option<String>) {
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: Some("contrib-ref".to_string()),
                    marker_denom,
                    mint_ldt_amount: None,
                    stored_access_grants: None,
                },
            )
            .unwrap();
        crate::state::CONTRIBUTION_REFERENCES
            .save(&mut ctx.deps.storage, "contrib-ref".to_string(), &0)
            .unwrap();
    }

    fn contributor_held_marker(ctx: &mut TestCtx, contract_permissions: Vec<i32>) {
        contributor_held_marker_state(
            ctx,
            contract_permissions,
            MarkerStatus::Active,
            MarkerType::Restricted,
        );
    }

    fn contributor_held_marker_state(
        ctx: &mut TestCtx,
        contract_permissions: Vec<i32>,
        status: MarkerStatus,
        marker_type: MarkerType,
    ) {
        let contributor = ctx.contributor.to_string();
        let contract = ctx.contract_addr();
        let other = ctx.stranger.to_string();
        ctx.mock_marker(
            MARKER_DENOM,
            &contributor,
            "1",
            vec![
                contract_access(&contract, contract_permissions),
                AccessGrant {
                    address: other,
                    permissions: vec![Access::Admin as i32],
                },
            ],
            status,
            marker_type,
        );
    }

    #[test]
    fn contributor_escrows_marker_and_stores_access_snapshot() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        contributor_held_marker(
            &mut ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap();

        assert_eq!(attr(&res, "method"), "submit_contribution");

        let stored = PENDING_CONTRIBUTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.marker_denom.as_deref(), Some(MARKER_DENOM));
        assert_eq!(
            PENDING_DENOMS
                .load(&ctx.deps.storage, MARKER_DENOM.to_string())
                .unwrap(),
            PendingDenomOwner::Contribution { id: 0 }
        );
        let grants = stored.stored_access_grants.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].address, ctx.stranger.to_string());

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].from_address, ctx.contributor.to_string());
        assert_eq!(
            transfers[0].to_address,
            format!("{MARKER_DENOM}.marker.account")
        );
        assert_eq!(
            transfers[0].amount.as_ref().map(|c| c.denom.as_str()),
            Some(MARKER_DENOM)
        );

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].removed_address, ctx.stranger.to_string());
        assert_eq!(deletes[0].denom, MARKER_DENOM);

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].access[0].address, ctx.contract_addr());
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
    fn binds_custody_to_checked_marker_when_decoy_denom_equals_address() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);

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
            "4",
            grants.clone(),
        );
        ctx.mock_restricted_marker(REAL_ADDRESS, &contributor, "4", grants);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            REAL_ADDRESS.to_string(),
        )
        .unwrap();

        let stored = PENDING_CONTRIBUTIONS.load(&ctx.deps.storage, 0).unwrap();
        assert_eq!(stored.marker_denom.as_deref(), Some(REAL_DENOM));

        let transfers: Vec<MsgTransferRequest> = typed_msgs(&res, MsgTransferRequest::TYPE_URL);
        assert_eq!(
            transfers[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((REAL_DENOM, "4"))
        );
        assert_eq!(transfers[0].to_address, REAL_ADDRESS);

        let deletes: Vec<MsgDeleteAccessRequest> =
            typed_msgs(&res, MsgDeleteAccessRequest::TYPE_URL);
        assert!(deletes.iter().all(|msg| msg.denom == REAL_DENOM));
    }

    #[test]
    fn rejects_escrowing_the_ldt_marker() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);

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
        let err = execute_submit_contribution(
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
        assert!(PENDING_CONTRIBUTIONS
            .load(&ctx.deps.storage, 0)
            .unwrap()
            .marker_denom
            .is_none());
    }

    #[test]
    fn rejects_escrowing_the_ldt_marker_supplied_as_its_address() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);

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
        let err = execute_submit_contribution(
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
    fn rejects_marker_whose_denom_parses_as_account_address() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);

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
        let err =
            execute_submit_contribution(ctx.deps.as_mut(), env, contributor_info, 0, decoy_address)
                .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomIsAddress { denom } if denom == address_shaped_denom
        ));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .unwrap()
            .marker_denom
            .is_none());
    }

    #[test]
    fn grants_full_custody_permissions_when_contract_already_has_admin() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        contributor_held_marker(
            &mut ctx,
            vec![Access::Admin as i32, Access::Withdraw as i32],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res = execute_submit_contribution(
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
        seed_pending(&mut ctx, None);
        contributor_held_marker_state(
            &mut ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
            MarkerStatus::Proposed,
            MarkerType::Restricted,
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_contribution(
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
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .unwrap()
            .marker_denom
            .is_none());
    }

    #[test]
    fn rejects_unrestricted_marker_before_writing_state() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        contributor_held_marker_state(
            &mut ctx,
            vec![Access::Admin as i32, Access::Transfer as i32],
            MarkerStatus::Active,
            MarkerType::Coin,
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_contribution(
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
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .unwrap()
            .marker_denom
            .is_none());
    }

    #[test]
    fn rejects_when_contract_has_only_transfer_access() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        contributor_held_marker(&mut ctx, vec![Access::Transfer as i32]);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .unwrap()
            .marker_denom
            .is_none());
    }

    #[test]
    fn rejects_second_submit_after_marker_is_escrowed() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, Some(MARKER_DENOM.to_string()));

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::ContributionDenomAlreadySet));
    }

    #[test]
    fn rejects_non_contributor() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        contributor_held_marker(&mut ctx, vec![Access::Transfer as i32]);

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            stranger_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedContributorAction));
    }

    #[test]
    fn rejects_when_contributor_does_not_hold_marker() {
        let mut ctx = TestCtx::new();
        seed_pending(&mut ctx, None);
        let contract = ctx.contract_addr();
        let stranger = ctx.stranger.to_string();
        ctx.mock_restricted_marker(
            MARKER_DENOM,
            &stranger,
            "1",
            vec![contract_access(&contract, vec![Access::Transfer as i32])],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_submit_contribution(
            ctx.deps.as_mut(),
            env,
            contributor_info,
            0,
            MARKER_DENOM.to_string(),
        )
        .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedContributorAction));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&TestCtx::unexpected_funds());
        let err =
            execute_submit_contribution(ctx.deps.as_mut(), env, info, 0, MARKER_DENOM.to_string())
                .unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }
}
