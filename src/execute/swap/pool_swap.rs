use crate::{
    error::ContractError,
    execute::assert_no_funds,
    pending::{require_source_not_foreign_pending_pool, ExceptPendingRecord},
    state::{CONFIGURATION, PENDING_SWAPS},
    util::{
        get_marker, get_marker_by_denom, get_nft, get_single_denom_holder_by_denom,
        validate_and_normalize_scope_uuids,
    },
};
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use itertools::Itertools;
use provwasm_std::types::{
    cosmos::base::v1beta1::Coin, provenance::marker::v1::MsgWithdrawRequest,
};

pub fn execute_pool_swap(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    swap_id: u64,
    removed_scope_uuids: Vec<String>,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Load the contract configuration
    let configuration = CONFIGURATION.load(deps.storage)?;

    // Verify that the sender is the authorized OTC address
    if configuration.otc_address != info.sender {
        return Err(ContractError::UnauthorizedOTC);
    }

    // Load the pending swap
    let swap = PENDING_SWAPS.load(deps.storage, swap_id)?;

    // Freeze the pool once the contributor has escrowed, and once OTC has marked
    // pooling complete. Re-opening is not a supported path.
    if swap.incoming_marker_denom.is_some() {
        return Err(ContractError::SwapIncomingDenomAlreadySet);
    }
    if swap.pooling_complete {
        return Err(ContractError::SwapPoolingAlreadyComplete);
    }

    // Validate and normalize the scope UUIDs provided for pooling and sort for consistent processing
    let mut scopes_to_pool = validate_and_normalize_scope_uuids(&removed_scope_uuids)?;
    scopes_to_pool.sort();

    // Get all the nft corresponding to the scope UUIDs
    let nfts = scopes_to_pool
        .iter()
        .map(|scope_uuid| get_nft(deps.as_ref(), scope_uuid))
        .collect::<Result<Vec<_>, ContractError>>()?;

    // Group the NFTs by their value owner for processing
    let grouped_nfts = nfts
        .into_iter()
        .into_group_map_by(|nft| nft.value_owner.clone());

    // Retrieve the pool marker and its base account for the swap
    let pool_marker = get_marker_by_denom(deps.as_ref(), swap.pool_denom.clone())?;
    let pool_marker_base_account = pool_marker.base_account.ok_or_else(|| {
        ContractError::MarkerQueryError("Error getting base account for marker".to_string())
    })?;

    let mut msg_withdraws: Vec<MsgWithdrawRequest> = grouped_nfts
        .into_iter()
        .map(|(owner, nfts)| {
            let marker = get_marker(deps.as_ref(), owner.clone())?;

            let address_balance =
                get_single_denom_holder_by_denom(deps.as_ref(), marker.denom.clone())?;
            if address_balance.address != env.contract.address.to_string() {
                return Err(ContractError::UnauthorizedScopeForSwap);
            }

            // Ensure that the source marker is not part of any other pending transaction
            require_source_not_foreign_pending_pool(
                deps.storage,
                &marker.denom,
                ExceptPendingRecord::Swap(swap_id),
            )?;

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
                denom: marker.denom.clone(),
                administrator: env.contract.address.to_string(),
                // Deposit into the pool marker
                to_address: pool_marker_base_account.address.to_string(),
                amount: coins,
            })
        })
        .collect::<Result<Vec<MsgWithdrawRequest>, ContractError>>()?;

    // Sort the withdraw requests by marker denom to ensure a deterministic order of messagesQ
    msg_withdraws.sort_by(|a, b| a.denom.cmp(&b.denom));

    Ok(Response::new()
        .add_messages(msg_withdraws)
        .add_attribute("method", "pool_swap")
        .add_attribute("swap_id", swap_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::{register_pending_denom, unregister_pending_denom};
    use crate::state::{
        PendingDenomOwner, PendingRedemption, PendingSwap, PENDING_REDEMPTIONS, PENDING_SWAPS,
    };
    use crate::testing::{typed_msgs, TestCtx, MARKER_DENOM, POOL_DENOM};
    use provwasm_std::types::provenance::marker::v1::MsgWithdrawRequest;

    const SCOPE_UUID: &str = "2e9e2078-2274-4289-be2c-6d70d46c23d3";
    const SCOPE_UUID_B: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    const SCOPE_ADDR: &str = "scope1qtestscopeaddr";
    const SCOPE_ADDR_B: &str = "scope1qotherscopeaddr";
    const OTHER_POOL: &str = "other.pool.marker";
    const SOURCE_A: &str = "aaa.source.marker";
    const SOURCE_Z: &str = "zzz.source.marker";

    fn seed_swap(
        ctx: &mut TestCtx,
        id: u64,
        pool_denom: &str,
        pooling_complete: bool,
        incoming: Option<&str>,
    ) {
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                id,
                &PendingSwap {
                    pool_denom: pool_denom.to_string(),
                    pooling_complete,
                    contributor_addr: ctx.contributor.clone(),
                    swap_ref: None,
                    mint_ldt_amount: 0,
                    burn_ldt_amount: 0,
                    incoming_marker_denom: incoming.map(str::to_string),
                    stored_incoming_access_grants: None,
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            pool_denom,
            PendingDenomOwner::Swap { id },
        )
        .unwrap();
        if let Some(incoming) = incoming {
            register_pending_denom(
                &mut ctx.deps.storage,
                incoming,
                PendingDenomOwner::Swap { id },
            )
            .unwrap();
        }
    }

    fn seed_open_swap(ctx: &mut TestCtx) {
        seed_swap(ctx, 0, POOL_DENOM, false, None);
    }

    fn mock_contract_held_marker(ctx: &mut TestCtx, denom: &str) {
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(denom, &contract, "1", vec![]);
    }

    fn mock_scope_on_marker(ctx: &mut TestCtx, marker_denom: &str) {
        let owner = format!("{marker_denom}.marker.account");
        ctx.mock_scope(SCOPE_UUID, &owner, SCOPE_ADDR);
    }

    #[test]
    fn rejects_non_otc() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            stranger_info,
            env,
            0,
            vec!["2e9e2078-2274-4289-be2c-6d70d46c23d3".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedOTC));
    }

    #[test]
    fn rejects_empty_scope_list() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(ctx.deps.as_mut(), otc_info, env, 0, vec![]).unwrap_err();
        assert!(matches!(err, ContractError::NoScopesProvided));
    }

    #[test]
    fn rejects_duplicate_scope_uuids() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        let scope = SCOPE_UUID.to_string();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![scope.clone(), scope],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::DuplicateScopeUuids));
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.otc_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_pool_swap(ctx.deps.as_mut(), info, env, 0, vec![]).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_scope_value_owned_by_another_pending_swap_pool() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        seed_swap(&mut ctx, 1, OTHER_POOL, false, None);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, OTHER_POOL);
        mock_scope_on_marker(&mut ctx, OTHER_POOL);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingSwap { swap_id: 1, .. }
        ));
    }

    #[test]
    fn rejects_scope_value_owned_by_another_pending_swap_incoming() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        seed_swap(&mut ctx, 1, OTHER_POOL, true, Some(MARKER_DENOM));
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, MARKER_DENOM);
        mock_scope_on_marker(&mut ctx, MARKER_DENOM);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingSwap { swap_id: 1, .. }
        ));
    }

    #[test]
    fn rejects_scope_value_owned_by_a_pending_redemption_pool() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                4,
                &PendingRedemption {
                    burn_ldt_amount: None,
                    pool_denom: OTHER_POOL.to_string(),
                    pooling_complete: false,
                    redemption_ref: None,
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            OTHER_POOL,
            PendingDenomOwner::Redemption { id: 4 },
        )
        .unwrap();
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, OTHER_POOL);
        mock_scope_on_marker(&mut ctx, OTHER_POOL);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingRedemption {
                redemption_id: 4,
                ..
            }
        ));
    }

    #[test]
    fn accepts_scope_on_cancelled_swap_pool_marker() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        seed_swap(&mut ctx, 1, OTHER_POOL, false, None);
        PENDING_SWAPS.remove(&mut ctx.deps.storage, 1);
        unregister_pending_denom(&mut ctx.deps.storage, OTHER_POOL);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, OTHER_POOL);
        mock_scope_on_marker(&mut ctx, OTHER_POOL);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap();

        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws.len(), 1);
        assert_eq!(withdraws[0].denom, OTHER_POOL);
        assert_eq!(
            withdraws[0].to_address,
            format!("{POOL_DENOM}.marker.account")
        );
        assert_eq!(withdraws[0].amount[0].denom, format!("nft/{SCOPE_ADDR}"));
    }

    #[test]
    fn rejects_after_incoming_marker_is_submitted() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, true, Some(MARKER_DENOM));

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::SwapIncomingDenomAlreadySet));
    }

    #[test]
    fn rejects_after_pooling_complete() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, true, None);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::SwapPoolingAlreadyComplete));
    }

    #[test]
    fn rejects_invalid_scope_uuid() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec!["not-a-uuid".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidScopeUuidFormat(_)));
    }

    #[test]
    fn rejects_case_insensitive_duplicate_scope_uuids() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string(), SCOPE_UUID.to_uppercase()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::DuplicateScopeUuids));
    }

    #[test]
    fn pools_contract_held_scopes_grouped_by_marker_and_sorted() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, SOURCE_A);
        mock_contract_held_marker(&mut ctx, SOURCE_Z);
        ctx.mock_scope(
            SCOPE_UUID,
            &format!("{SOURCE_Z}.marker.account"),
            SCOPE_ADDR,
        );
        ctx.mock_scope(
            SCOPE_UUID_B,
            &format!("{SOURCE_A}.marker.account"),
            SCOPE_ADDR_B,
        );

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_uppercase(), SCOPE_UUID_B.to_string()],
        )
        .unwrap();

        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws.len(), 2);
        assert_eq!(withdraws[0].denom, SOURCE_A);
        assert_eq!(withdraws[1].denom, SOURCE_Z);
        assert!(withdraws.iter().all(|msg| {
            msg.administrator == ctx.contract_addr()
                && msg.to_address == format!("{POOL_DENOM}.marker.account")
        }));
        assert_eq!(withdraws[0].amount[0].denom, format!("nft/{SCOPE_ADDR_B}"));
        assert_eq!(withdraws[1].amount[0].denom, format!("nft/{SCOPE_ADDR}"));
    }

    #[test]
    fn pools_several_scopes_on_one_marker_as_one_sorted_withdraw() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, SOURCE_A);
        let owner = format!("{SOURCE_A}.marker.account");
        ctx.mock_scope(SCOPE_UUID, &owner, SCOPE_ADDR);
        ctx.mock_scope(SCOPE_UUID_B, &owner, SCOPE_ADDR_B);

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let res = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string(), SCOPE_UUID_B.to_string()],
        )
        .unwrap();

        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert_eq!(withdraws.len(), 1);
        assert_eq!(withdraws[0].denom, SOURCE_A);
        assert_eq!(
            withdraws[0]
                .amount
                .iter()
                .map(|coin| coin.denom.clone())
                .collect::<Vec<_>>(),
            vec![format!("nft/{SCOPE_ADDR_B}"), format!("nft/{SCOPE_ADDR}"),]
        );
    }

    #[test]
    fn rejects_scope_the_contract_does_not_hold() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        let stranger = ctx.stranger.to_string();
        ctx.mock_restricted_marker(SOURCE_A, &stranger, "1", vec![]);
        ctx.mock_scope(
            SCOPE_UUID,
            &format!("{SOURCE_A}.marker.account"),
            SCOPE_ADDR,
        );

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::UnauthorizedScopeForSwap));
    }

    #[test]
    fn rejects_scope_value_owned_by_a_pending_contribution_marker() {
        let mut ctx = TestCtx::new();
        seed_open_swap(&mut ctx);
        mock_contract_held_marker(&mut ctx, POOL_DENOM);
        mock_contract_held_marker(&mut ctx, SOURCE_A);
        mock_scope_on_marker(&mut ctx, SOURCE_A);
        register_pending_denom(
            &mut ctx.deps.storage,
            SOURCE_A,
            PendingDenomOwner::Contribution { id: 9 },
        )
        .unwrap();

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err = execute_pool_swap(
            ctx.deps.as_mut(),
            otc_info,
            env,
            0,
            vec![SCOPE_UUID.to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingContribution {
                contribution_id: 9,
                ..
            }
        ));
    }
}
