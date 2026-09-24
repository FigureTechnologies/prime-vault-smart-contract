use crate::error::ContractError;
use crate::msg::MigrateMsg;
use crate::state::{
    CONTRACT_NAME, CONTRACT_VERSION, PENDING_CONTRIBUTIONS, PENDING_DENOMS, PENDING_REDEMPTIONS,
    PENDING_SWAPS,
};
use cosmwasm_std::{DepsMut, Env, Order, Response, StdResult, Storage};
use cw2::{get_contract_version, set_contract_version};
use cw_storage_plus::Map;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Stored contract version that may migrate onto this code, besides the current version.
pub const PREVIOUS_CONTRACT_VERSION: &str = "0.3.0";

pub fn migrate_contract(
    deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
) -> Result<Response, ContractError> {
    // V
    let stored = get_contract_version(deps.storage)?;
    if stored.contract != CONTRACT_NAME {
        return Err(ContractError::CannotMigrateContract {
            expected: CONTRACT_NAME.to_string(),
            found: stored.contract,
        });
    }

    if stored.version != PREVIOUS_CONTRACT_VERSION && stored.version != CONTRACT_VERSION {
        return Err(ContractError::CannotMigrateFromVersion {
            version: stored.version,
        });
    }

    // Check for any pending contributions, redemptions, or swaps before proceeding with the migration
    let contributions = pending_key_count(deps.storage, PENDING_CONTRIBUTIONS)?;
    let redemptions = pending_key_count(deps.storage, PENDING_REDEMPTIONS)?;
    let swaps = pending_key_count(deps.storage, PENDING_SWAPS)?;
    if contributions > 0 || redemptions > 0 || swaps > 0 {
        return Err(ContractError::PendingRecordsExist {
            contributions,
            redemptions,
            swaps,
        });
    }

    // With the pending maps confirmed empty, any PENDING_DENOMS entry is stale by definition:
    // entries are only written alongside a record. Clear them rather than refusing to migrate,
    // because unregister_pending_denom is a silent no-op on missing keys, so a leaked entry gives
    // no signal and has no other removal route — it would fence that denom permanently. The count
    // is emitted so a leak surfaces as an observable event instead of being silently repaired.
    let cleared_pending_denoms = clear_pending_denoms(deps.storage)?;

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new()
        .add_attribute("method", "migrate")
        .add_attribute("previous_version", stored.version)
        .add_attribute("new_version", CONTRACT_VERSION)
        .add_attribute("cleared_pending_denoms", cleared_pending_denoms.to_string()))
}

fn clear_pending_denoms(storage: &mut dyn Storage) -> StdResult<u32> {
    let denoms = PENDING_DENOMS
        .keys(storage, None, None, Order::Ascending)
        .collect::<StdResult<Vec<_>>>()?;
    let count = denoms.len() as u32;
    for denom in denoms {
        PENDING_DENOMS.remove(storage, denom);
    }
    Ok(count)
}

fn pending_key_count<T>(storage: &dyn Storage, map: Map<u64, T>) -> StdResult<u32>
where
    T: Serialize + DeserializeOwned,
{
    Ok(map
        .keys(storage, None, None, Order::Ascending)
        .collect::<StdResult<Vec<_>>>()?
        .len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instantiate::instantiate_contract::instantiate_contract;
    use crate::msg::InstantiateContractMsg;
    use crate::state::{
        PendingContribution, PendingRedemption, PendingSwap, CONTRACT_NAME, CONTRACT_VERSION,
        PENDING_CONTRIBUTIONS, PENDING_REDEMPTIONS, PENDING_SWAPS,
    };
    use crate::testing::{TestCtx, LDT_DENOM};
    use cw2::{get_contract_version, set_contract_version};

    fn seed_v0_3_contract(ctx: &mut TestCtx) {
        instantiate_contract(
            ctx.deps.as_mut(),
            ctx.env.clone(),
            InstantiateContractMsg {
                otc_address: ctx.otc.to_string(),
                ldt_denom: LDT_DENOM.to_string(),
                redemption_marker_admin: ctx.redemption_admin.to_string(),
                app_metadata: None,
            },
        )
        .unwrap();
        set_contract_version(
            &mut ctx.deps.storage,
            CONTRACT_NAME,
            PREVIOUS_CONTRACT_VERSION,
        )
        .unwrap();
    }

    fn dummy_contribution(ctx: &TestCtx) -> PendingContribution {
        PendingContribution {
            contributor_addr: ctx.contributor.clone(),
            contribution_ref: None,
            marker_denom: None,
            mint_ldt_amount: None,
            stored_access_grants: None,
        }
    }

    fn dummy_redemption(ctx: &TestCtx) -> PendingRedemption {
        PendingRedemption {
            burn_ldt_amount: None,
            pool_denom: "pool.marker".to_string(),
            pooling_complete: false,
            redemption_ref: None,
            redeemer_addr: ctx.redeemer.clone(),
        }
    }

    fn dummy_swap(ctx: &TestCtx) -> PendingSwap {
        PendingSwap {
            pool_denom: "pool.marker".to_string(),
            pooling_complete: false,
            contributor_addr: ctx.contributor.clone(),
            swap_ref: None,
            mint_ldt_amount: 0,
            burn_ldt_amount: 0,
            incoming_marker_denom: None,
            stored_incoming_access_grants: None,
        }
    }

    /// Removing a field from PendingContribution or PendingSwap needs no data migration because
    /// migrate refuses to run while any record exists. That holds even for records written by an
    /// older schema: pending_key_count reads keys only, so the gate reports PendingRecordsExist
    /// rather than failing to deserialize a record the current struct no longer matches.
    #[test]
    fn detects_pending_records_written_by_an_older_schema() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);

        // A v0.3.0 contribution, including the contract_permissions_before_submit field that no
        // longer exists on the struct.
        let legacy = br#"{"contributor_addr":"legacy","contribution_ref":null,"marker_denom":null,"stored_access_grants":null,"contract_permissions_before_submit":[1,2]}"#;
        let key = PENDING_CONTRIBUTIONS.key(0).to_vec();
        ctx.deps.storage.set(&key, legacy);

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();

        assert!(matches!(
            err,
            ContractError::PendingRecordsExist {
                contributions: 1,
                ..
            }
        ));
    }

    #[test]
    fn clears_leaked_pending_denoms_and_reports_the_count() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);

        // A denom left behind by an earlier bug: no pending record references it, and
        // unregister_pending_denom is a no-op on missing keys, so nothing else can remove it.
        PENDING_DENOMS
            .save(
                &mut ctx.deps.storage,
                "leaked.marker".to_string(),
                &crate::state::PendingDenomOwner::Swap { id: 9 },
            )
            .unwrap();

        let res =
            migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default()).unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "cleared_pending_denoms")
                .map(|a| a.value.as_str()),
            Some("1")
        );
        assert!(PENDING_DENOMS
            .may_load(&ctx.deps.storage, "leaked.marker".to_string())
            .unwrap()
            .is_none());
    }

    #[test]
    fn reports_zero_cleared_denoms_when_the_index_is_clean() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);

        let res =
            migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default()).unwrap();

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "cleared_pending_denoms")
                .map(|a| a.value.as_str()),
            Some("0")
        );
    }

    #[test]
    fn rejects_version_below_0_3_0() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        set_contract_version(&mut ctx.deps.storage, CONTRACT_NAME, "0.2.0").unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::CannotMigrateFromVersion { version } if version == "0.2.0"
        ));
    }

    #[test]
    fn rejects_version_0_1_0() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        set_contract_version(&mut ctx.deps.storage, CONTRACT_NAME, "0.1.0").unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::CannotMigrateFromVersion { version } if version == "0.1.0"
        ));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        set_contract_version(&mut ctx.deps.storage, CONTRACT_NAME, "0.4.0").unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::CannotMigrateFromVersion { version } if version == "0.4.0"
        ));
    }

    #[test]
    fn rejects_wrong_contract_name() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        set_contract_version(&mut ctx.deps.storage, "other-contract", "0.3.0").unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(err, ContractError::CannotMigrateContract { .. }));
    }

    #[test]
    fn rejects_pending_contribution() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        let contribution = dummy_contribution(&ctx);
        PENDING_CONTRIBUTIONS
            .save(&mut ctx.deps.storage, 0, &contribution)
            .unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::PendingRecordsExist {
                contributions: 1,
                redemptions: 0,
                swaps: 0,
            }
        ));
    }

    #[test]
    fn rejects_pending_redemption() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        let redemption = dummy_redemption(&ctx);
        PENDING_REDEMPTIONS
            .save(&mut ctx.deps.storage, 0, &redemption)
            .unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::PendingRecordsExist {
                contributions: 0,
                redemptions: 1,
                swaps: 0,
            }
        ));
    }

    #[test]
    fn rejects_pending_swap() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        let swap = dummy_swap(&ctx);
        PENDING_SWAPS.save(&mut ctx.deps.storage, 0, &swap).unwrap();

        let err = migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ContractError::PendingRecordsExist {
                contributions: 0,
                redemptions: 0,
                swaps: 1,
            }
        ));
    }

    #[test]
    fn migrates_from_0_3_0() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);

        let res =
            migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default()).unwrap();

        let ver = get_contract_version(&ctx.deps.storage).unwrap();
        assert_eq!(ver.contract, CONTRACT_NAME);
        assert_eq!(ver.version, CONTRACT_VERSION);
        assert!(res.messages.is_empty());
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "previous_version")
                .map(|a| a.value.as_str()),
            Some(PREVIOUS_CONTRACT_VERSION)
        );
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "new_version")
                .map(|a| a.value.as_str()),
            Some(CONTRACT_VERSION)
        );
    }

    #[test]
    fn idempotent_when_already_on_current_version() {
        let mut ctx = TestCtx::new();
        seed_v0_3_contract(&mut ctx);
        set_contract_version(&mut ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION).unwrap();

        migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default()).unwrap();
        let res =
            migrate_contract(ctx.deps.as_mut(), ctx.env.clone(), MigrateMsg::default()).unwrap();

        assert!(res.messages.is_empty());
        let ver = get_contract_version(&ctx.deps.storage).unwrap();
        assert_eq!(ver.version, CONTRACT_VERSION);
    }
}
