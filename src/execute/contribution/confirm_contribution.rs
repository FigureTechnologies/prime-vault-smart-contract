use crate::error::ContractError;
use crate::execute::assert_no_funds;
use crate::pending::unregister_pending_denom;
use crate::state::{CONFIGURATION, CONTRIBUTION_REFERENCES, PENDING_CONTRIBUTIONS};
use crate::util::{
    require_marker_permissions, resolve_stored_marker_holding, CONTRACT_CUSTODY_PERMISSIONS,
};
use cosmwasm_std::{CosmosMsg, DepsMut, Env, MessageInfo, Response};
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{MsgMintRequest, MsgWithdrawRequest};

/// Settles a priced contribution on the contributor's authority: mints the offered LDT to them
/// and takes the escrowed marker into contract custody.
///
/// The contributor supplies the price they agreed to. If the OTC re-priced the contribution
/// after the contributor read it, this call fails rather than settling at the newer figure, so
/// assets never move at an amount they did not confirm.
pub fn execute_confirm_contribution(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    contribution_id: u64,
    expected_mint_ldt_amount: u64,
) -> Result<Response, ContractError> {
    // Ensure no funds are sent with this message
    assert_no_funds(&info)?;

    // Load the current configuration and the pending contribution from state
    let configuration = CONFIGURATION.load(deps.storage)?;
    let contribution = PENDING_CONTRIBUTIONS.load(deps.storage, contribution_id)?;

    // Only the contributor whose assets are escrowed may confirm.
    if info.sender != contribution.contributor_addr {
        return Err(ContractError::UnauthorizedContributorAction);
    }

    // Get the number of ldt tokens to mint based on the contribution's pricing
    let mint_ldt_amount = contribution
        .mint_ldt_amount
        .ok_or(ContractError::ContributionNotPriced)?;

    // Pin the offer the contributor actually read. Pricing is re-callable, so without this a
    // concurrent re-price could settle the contribution at an amount they never agreed to.
    if expected_mint_ldt_amount != mint_ldt_amount {
        return Err(ContractError::ContributionPriceMismatch {
            expected: expected_mint_ldt_amount,
            actual: mint_ldt_amount,
        });
    }

    // Get the contribution denom for the pending transaction
    let contribution_denom = contribution
        .marker_denom
        .clone()
        .ok_or(ContractError::ContributionDenomNotSet)?
        .to_string();

    // Read the marker and the balance
    let (marker_account, balance) =
        resolve_stored_marker_holding(deps.as_ref(), contribution_denom.clone())?;

    // Refuse to mint until the queried grant already includes Admin, Withdraw, Deposit, and
    // Transfer. Submit grants this set, but confirm must not mint against a Transfer-only
    // (or otherwise incomplete) marker if that grant never landed.
    require_marker_permissions(
        &marker_account.access_control,
        env.contract.address.as_str(),
        &CONTRACT_CUSTODY_PERMISSIONS,
    )?;

    // Mint the LDT the OTC offered and the contributor confirmed. A zero offer is a valid
    // outcome the contributor may still confirm, in which case there is nothing to mint.
    let mint_msgs = if mint_ldt_amount > 0 {
        vec![
            CosmosMsg::from(MsgMintRequest {
                administrator: env.contract.address.to_string(),
                amount: Option::Some(Coin {
                    denom: configuration.ldt_denom.to_string(),
                    amount: mint_ldt_amount.to_string(),
                }),
            }),
            // Withdraw the minted LDT tokens to the marker holder (contributor)
            CosmosMsg::from(MsgWithdrawRequest {
                denom: configuration.ldt_denom.to_string(),
                administrator: env.contract.address.to_string(),
                to_address: contribution.contributor_addr.to_string(), // Withdraw to the marker holder (contributor)
                amount: vec![Coin {
                    denom: configuration.ldt_denom.to_string(),
                    amount: mint_ldt_amount.to_string(),
                }],
            }),
        ]
    } else {
        vec![]
    };

    // Withdraw the marker's coins to the contract
    let withdraw_msg = CosmosMsg::from(MsgWithdrawRequest {
        denom: contribution_denom.clone(),
        administrator: env.contract.address.to_string(),
        to_address: env.contract.address.to_string(),
        amount: vec![Coin {
            denom: contribution_denom.clone(),
            amount: balance.amount,
        }],
    });

    // Clear out the contribution details since this contribution has been settled.
    unregister_pending_denom(deps.storage, &contribution_denom);
    PENDING_CONTRIBUTIONS.remove(deps.storage, contribution_id);
    if let Some(ref contribution_ref) = contribution.contribution_ref {
        CONTRIBUTION_REFERENCES.remove(deps.storage, contribution_ref.to_string());
    }

    let response = Response::new()
        .add_messages(mint_msgs)
        .add_message(withdraw_msg)
        .add_attribute("method", "confirm_contribution")
        .add_attribute("contribution_id", contribution_id.to_string())
        .add_attribute("marker_denom", contribution_denom)
        .add_attribute("mint_ldt_amount", mint_ldt_amount.to_string());

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::register_pending_denom;
    use crate::state::{
        PendingContribution, PendingDenomOwner, CONTRIBUTION_REFERENCES, PENDING_CONTRIBUTIONS,
        PENDING_DENOMS,
    };
    use crate::testing::{attr, contract_access, typed_msgs, TestCtx, LDT_DENOM, MARKER_DENOM};
    use crate::util::CONTRACT_CUSTODY_PERMISSIONS;
    use provwasm_std::types::provenance::marker::v1::{
        Access, MsgAddAccessRequest, MsgMintRequest, MsgWithdrawRequest,
    };

    fn seed_priced(ctx: &mut TestCtx, price: Option<u64>) {
        seed_priced_with_permissions(ctx, price, CONTRACT_CUSTODY_PERMISSIONS.to_vec());
    }

    fn seed_priced_with_permissions(ctx: &mut TestCtx, price: Option<u64>, permissions: Vec<i32>) {
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: Some("contrib-ref".to_string()),
                    marker_denom: Some(MARKER_DENOM.to_string()),
                    mint_ldt_amount: price,
                    stored_access_grants: None,
                },
            )
            .unwrap();
        CONTRIBUTION_REFERENCES
            .save(&mut ctx.deps.storage, "contrib-ref".to_string(), &0)
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            MARKER_DENOM,
            PendingDenomOwner::Contribution { id: 0 },
        )
        .unwrap();

        let marker_account = format!("{MARKER_DENOM}.marker.account");
        let contract = ctx.contract_addr();
        ctx.mock_restricted_marker(
            MARKER_DENOM,
            &marker_account,
            "1",
            vec![contract_access(&contract, permissions)],
        );
    }

    #[test]
    fn contributor_confirms_priced_contribution_and_receives_minted_ldt() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, Some(250));

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res =
            execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 250).unwrap();

        assert_eq!(attr(&res, "method"), "confirm_contribution");
        assert_eq!(attr(&res, "mint_ldt_amount"), "250");

        let mints: Vec<MsgMintRequest> = typed_msgs(&res, MsgMintRequest::TYPE_URL);
        assert_eq!(mints.len(), 1);
        assert_eq!(
            mints[0]
                .amount
                .as_ref()
                .map(|c| (c.denom.as_str(), c.amount.as_str())),
            Some((LDT_DENOM, "250"))
        );

        let withdraws: Vec<MsgWithdrawRequest> = typed_msgs(&res, MsgWithdrawRequest::TYPE_URL);
        assert!(withdraws.iter().any(|msg| {
            msg.denom == LDT_DENOM
                && msg.to_address == ctx.contributor.to_string()
                && msg
                    .amount
                    .iter()
                    .any(|c| c.denom == LDT_DENOM && c.amount == "250")
        }));
        assert!(withdraws
            .iter()
            .any(|msg| { msg.denom == MARKER_DENOM && msg.to_address == ctx.contract_addr() }));

        let grants: Vec<MsgAddAccessRequest> = typed_msgs(&res, MsgAddAccessRequest::TYPE_URL);
        assert!(grants.is_empty());

        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
        assert!(CONTRIBUTION_REFERENCES
            .may_load(&ctx.deps.storage, "contrib-ref".to_string())
            .unwrap()
            .is_none());
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, MARKER_DENOM.to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn confirming_a_zero_offer_settles_without_minting() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, Some(0));

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let res =
            execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 0).unwrap();

        let mints: Vec<MsgMintRequest> = typed_msgs(&res, MsgMintRequest::TYPE_URL);
        assert!(mints.is_empty());
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_confirm_before_the_otc_has_priced() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, None);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 250)
            .unwrap_err();

        assert!(matches!(err, ContractError::ContributionNotPriced));
    }

    #[test]
    fn rejects_confirm_when_the_otc_repriced_first() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, Some(1));

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 250)
            .unwrap_err();

        assert!(matches!(
            err,
            ContractError::ContributionPriceMismatch {
                expected: 250,
                actual: 1,
            }
        ));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }

    #[test]
    fn rejects_otc_attempting_to_settle() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, Some(250));

        let env = ctx.env.clone();
        let otc_info = ctx.otc_info();
        let err =
            execute_confirm_contribution(ctx.deps.as_mut(), env, otc_info, 0, 250).unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedContributorAction));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }

    #[test]
    fn rejects_stranger() {
        let mut ctx = TestCtx::new();
        seed_priced(&mut ctx, Some(250));

        let env = ctx.env.clone();
        let stranger_info = ctx.stranger_info();
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, stranger_info, 0, 250)
            .unwrap_err();

        assert!(matches!(err, ContractError::UnauthorizedContributorAction));
    }

    #[test]
    fn rejects_when_contract_lacks_admin_even_with_withdraw() {
        let mut ctx = TestCtx::new();
        seed_priced_with_permissions(
            &mut ctx,
            Some(250),
            vec![
                Access::Withdraw as i32,
                Access::Deposit as i32,
                Access::Transfer as i32,
            ],
        );

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 250)
            .unwrap_err();

        assert!(matches!(err, ContractError::InsufficientMarkerAccess));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }

    #[test]
    fn rejects_unexpected_funds() {
        let mut ctx = TestCtx::new();
        let env = ctx.env.clone();
        let info = ctx.contributor_info_with_funds(&TestCtx::unexpected_funds());
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, info, 0, 250).unwrap_err();
        assert!(matches!(err, ContractError::InvalidFundsSent));
    }

    #[test]
    fn rejects_when_stored_denom_resolves_to_a_different_marker() {
        let mut ctx = TestCtx::new();
        let victim_address = ctx.deps.api.addr_make("victim-marker").to_string();
        PENDING_CONTRIBUTIONS
            .save(
                &mut ctx.deps.storage,
                0,
                &PendingContribution {
                    contributor_addr: ctx.contributor.clone(),
                    contribution_ref: Some("contrib-ref".to_string()),
                    marker_denom: Some(victim_address.clone()),
                    mint_ldt_amount: Some(250),
                    stored_access_grants: None,
                },
            )
            .unwrap();
        CONTRIBUTION_REFERENCES
            .save(&mut ctx.deps.storage, "contrib-ref".to_string(), &0)
            .unwrap();

        let contract = ctx.contract_addr();
        let contributor = ctx.contributor.to_string();
        ctx.mock_restricted_marker_with_address(
            MARKER_DENOM,
            &victim_address,
            &format!("{MARKER_DENOM}.marker.account"),
            "1",
            vec![contract_access(
                &contract,
                CONTRACT_CUSTODY_PERMISSIONS.to_vec(),
            )],
        );
        ctx.mock_restricted_marker(&victim_address, &contributor, "1", vec![]);

        let env = ctx.env.clone();
        let contributor_info = ctx.contributor_info();
        let err = execute_confirm_contribution(ctx.deps.as_mut(), env, contributor_info, 0, 250)
            .unwrap_err();

        assert!(matches!(
            err,
            ContractError::MarkerDenomResolutionMismatch {
                requested_denom,
                resolved_denom,
            } if requested_denom == victim_address && resolved_denom == MARKER_DENOM
        ));
        assert!(PENDING_CONTRIBUTIONS
            .may_load(&ctx.deps.storage, 0)
            .unwrap()
            .is_some());
    }
}
