use cosmwasm_std::Storage;

use crate::error::ContractError;
use crate::state::{PendingDenomOwner, PENDING_DENOMS};

/// Identifies the pending record currently being pooled so its own denoms are not fenced.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExceptPendingRecord {
    Swap(u64),
    Redemption(u64),
}

/// Claims `denom` for `owner`. Errors if another live record already indexed it.
pub fn register_pending_denom(
    storage: &mut dyn Storage,
    denom: impl Into<String>,
    owner: PendingDenomOwner,
) -> Result<(), ContractError> {
    let denom = denom.into();
    if PENDING_DENOMS.may_load(storage, denom.clone())?.is_some() {
        return Err(ContractError::PendingDenomAlreadyInUse { denom });
    }
    PENDING_DENOMS.save(storage, denom, &owner)?;
    Ok(())
}

/// Drops the reverse-index entry for `denom`. No-op if it is already absent
/// (tests that seed pending records without the index still cancel/confirm).
pub fn unregister_pending_denom(storage: &mut dyn Storage, denom: impl AsRef<str>) {
    PENDING_DENOMS.remove(storage, denom.as_ref().to_string());
}

/// Rejects a pooling source marker that belongs to any live pending record other
/// than `except`. Cancelled records are removed from the index, so leftover pool
/// markers remain valid re-pool sources.
pub fn require_source_not_foreign_pending_pool(
    storage: &dyn Storage,
    source_denom: &str,
    except: ExceptPendingRecord,
) -> Result<(), ContractError> {
    match PENDING_DENOMS.may_load(storage, source_denom.to_string())? {
        None => Ok(()),
        Some(PendingDenomOwner::Swap { id }) => {
            if matches!(except, ExceptPendingRecord::Swap(except_id) if except_id == id) {
                Ok(())
            } else {
                Err(ContractError::ScopeOwnedByPendingSwap {
                    denom: source_denom.to_string(),
                    swap_id: id,
                })
            }
        }
        Some(PendingDenomOwner::Redemption { id }) => {
            if matches!(except, ExceptPendingRecord::Redemption(except_id) if except_id == id) {
                Ok(())
            } else {
                Err(ContractError::ScopeOwnedByPendingRedemption {
                    denom: source_denom.to_string(),
                    redemption_id: id,
                })
            }
        }
        Some(PendingDenomOwner::Contribution { id }) => {
            Err(ContractError::ScopeOwnedByPendingContribution {
                denom: source_denom.to_string(),
                contribution_id: id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingRedemption, PendingSwap, PENDING_REDEMPTIONS, PENDING_SWAPS};
    use crate::testing::{TestCtx, MARKER_DENOM, POOL_DENOM};

    const OTHER_POOL: &str = "other.pool.marker";

    fn seed_swap(ctx: &mut TestCtx, id: u64, pool_denom: &str, incoming: Option<&str>) {
        PENDING_SWAPS
            .save(
                &mut ctx.deps.storage,
                id,
                &PendingSwap {
                    pool_denom: pool_denom.to_string(),
                    pooling_complete: false,
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

    fn seed_redemption(ctx: &mut TestCtx, id: u64, pool_denom: &str) {
        PENDING_REDEMPTIONS
            .save(
                &mut ctx.deps.storage,
                id,
                &PendingRedemption {
                    burn_ldt_amount: None,
                    pool_denom: pool_denom.to_string(),
                    pooling_complete: false,
                    redemption_ref: None,
                    redeemer_addr: ctx.redeemer.clone(),
                },
            )
            .unwrap();
        register_pending_denom(
            &mut ctx.deps.storage,
            pool_denom,
            PendingDenomOwner::Redemption { id },
        )
        .unwrap();
    }

    #[test]
    fn allows_source_that_is_the_record_being_pooled() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);

        require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            POOL_DENOM,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap();
    }

    #[test]
    fn rejects_source_owned_by_another_pending_swap_pool() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);
        seed_swap(&mut ctx, 1, OTHER_POOL, None);

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            OTHER_POOL,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingSwap { swap_id: 1, .. }
        ));
    }

    #[test]
    fn rejects_source_owned_by_another_pending_swap_incoming() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);
        seed_swap(&mut ctx, 1, OTHER_POOL, Some(MARKER_DENOM));

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            MARKER_DENOM,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingSwap { swap_id: 1, .. }
        ));
    }

    #[test]
    fn rejects_source_owned_by_a_pending_redemption_pool() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);
        seed_redemption(&mut ctx, 3, OTHER_POOL);

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            OTHER_POOL,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingRedemption {
                redemption_id: 3,
                ..
            }
        ));
    }

    #[test]
    fn allows_source_from_cancelled_record_pool() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);
        seed_swap(&mut ctx, 1, OTHER_POOL, None);
        PENDING_SWAPS.remove(&mut ctx.deps.storage, 1);
        unregister_pending_denom(&mut ctx.deps.storage, OTHER_POOL);

        require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            OTHER_POOL,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap();
    }

    #[test]
    fn redemption_except_allows_own_pool_but_not_a_live_swap() {
        let mut ctx = TestCtx::new();
        seed_redemption(&mut ctx, 0, POOL_DENOM);
        seed_swap(&mut ctx, 2, OTHER_POOL, None);

        require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            POOL_DENOM,
            ExceptPendingRecord::Redemption(0),
        )
        .unwrap();

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            OTHER_POOL,
            ExceptPendingRecord::Redemption(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingSwap { swap_id: 2, .. }
        ));
    }

    #[test]
    fn rejects_source_owned_by_a_pending_contribution() {
        let mut ctx = TestCtx::new();
        seed_swap(&mut ctx, 0, POOL_DENOM, None);
        register_pending_denom(
            &mut ctx.deps.storage,
            MARKER_DENOM,
            PendingDenomOwner::Contribution { id: 8 },
        )
        .unwrap();

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            MARKER_DENOM,
            ExceptPendingRecord::Swap(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingContribution {
                contribution_id: 8,
                ..
            }
        ));

        let err = require_source_not_foreign_pending_pool(
            &ctx.deps.storage,
            MARKER_DENOM,
            ExceptPendingRecord::Redemption(0),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::ScopeOwnedByPendingContribution {
                contribution_id: 8,
                ..
            }
        ));
    }

    #[test]
    fn register_rejects_duplicate_denom() {
        let mut ctx = TestCtx::new();
        register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Swap { id: 0 },
        )
        .unwrap();
        let err = register_pending_denom(
            &mut ctx.deps.storage,
            POOL_DENOM,
            PendingDenomOwner::Redemption { id: 1 },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::PendingDenomAlreadyInUse { denom } if denom == POOL_DENOM
        ));
    }
}
