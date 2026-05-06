use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, StdResult, Storage};
use cw2::ContractVersion;
use cw_storage_plus::{Item, Map};

use crate::error::ContractError;

/// Contributor attribution while a scope is in the LDT pool: the address that finalized the
/// contribution (marker coin holder) for each scope UUID. Cleared when that scope is redeemed and
/// transferred out via [`execute_finalize_redemption`](crate::execute::redemption::finalize_redemption::execute_finalize_redemption).

// version info for migration info
pub const CONTRACT_NAME: &str = "crates.io:loan-dicer";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MAX_CONTRACT_APP_METADATA_KIND_LEN: usize = 64;
pub const MAX_CONTRACT_APP_METADATA_DATA_LEN: usize = 1024;
pub const MAX_CONTRIBUTION_REF_LEN: usize = 128;
pub const MAX_REDEMPTION_REF_LEN: usize = 128;

/// App-defined metadata for this contract instance (not interpreted by the contract).
#[cw_serde]
pub struct ContractAppMetadata {
    /// Optional discriminator, e.g. external transaction flow type.
    pub kind: Option<String>,
    /// Opaque payload (convention: JSON string).
    pub data: Option<String>,
}

pub fn validate_contract_app_metadata(
    metadata: &Option<ContractAppMetadata>,
) -> Result<(), ContractError> {
    if let Some(m) = metadata {
        if let Some(ref k) = m.kind {
            if k.len() > MAX_CONTRACT_APP_METADATA_KIND_LEN {
                return Err(ContractError::ContractAppMetadataTooLong { field: "kind" });
            }
        }
        if let Some(ref d) = m.data {
            if d.len() > MAX_CONTRACT_APP_METADATA_DATA_LEN {
                return Err(ContractError::ContractAppMetadataTooLong { field: "data" });
            }
        }
    }
    Ok(())
}

#[cw_serde]
pub struct PendingContribution {
    pub mint_ldt_amount: u64,
    pub scope_uuids: Vec<String>,
    pub contribution_ref: Option<String>,
}

#[cw_serde]
pub struct PendingRedemption {
    pub burn_ldt_amount: u64,
    pub scope_uuids: Vec<String>,
    pub pool_denom: String,
    pub pooling_complete: bool,
    pub redemption_ref: Option<String>,
    pub redeemer_addr: Addr,
}

#[cw_serde]
pub struct Configuration {
    pub otc_address: Addr,
    pub ldt_denom: String,
    pub redemption_marker_admin: Addr,
    #[serde(default)]
    pub app_metadata: Option<ContractAppMetadata>,
}

// state items
// Contract Info uses the key "contract_info" per the cw2 spec
pub const CONTRACT_INFO: Item<ContractVersion> = Item::new("contract_info");
pub const PENDING_CONTRIBUTIONS: Map<u64, PendingContribution> = Map::new("contributions");
pub const PENDING_REDEMPTIONS: Map<u64, PendingRedemption> = Map::new("redemptions");
pub const CONTRIBUTION_COUNTER: Item<u64> = Item::new("contribution_counter");
pub const REDEMPTION_COUNTER: Item<u64> = Item::new("redemption_counter");
pub const CONFIGURATION: Item<Configuration> = Item::new("configuration");
pub const CONTRIBUTION_REFERENCES: Map<String, u64> = Map::new("contribution_references");
pub const REDEMPTION_REFERENCES: Map<String, u64> = Map::new("redemption_references");
/// scope_uuid (normalized) → original contributor (finalize-time marker holder)
pub const SCOPE_CONTRIBUTOR: Map<String, Addr> = Map::new("scope_contributor");

/// Removes contributor attribution for the given scope UUIDs.
pub fn clear_contributor_attribution_for_scopes(
    storage: &mut dyn Storage,
    scope_uuids: &[String],
) -> StdResult<()> {
    for scope_uuid in scope_uuids {
        SCOPE_CONTRIBUTOR.remove(storage, scope_uuid.clone());
    }
    Ok(())
}

#[cfg(test)]
mod clear_attribution_tests {
    use super::*;
    use cosmwasm_std::MemoryStorage;

    #[test]
    fn clears_scope_contributor_map() {
        let mut storage = MemoryStorage::new();
        let contributor = Addr::unchecked("contributor");
        let scope = "2e9e2078-2274-4289-be2c-6d70d46c23d3".to_string();
        SCOPE_CONTRIBUTOR
            .save(&mut storage, scope.clone(), &contributor)
            .unwrap();

        clear_contributor_attribution_for_scopes(&mut storage, std::slice::from_ref(&scope))
            .unwrap();

        assert!(SCOPE_CONTRIBUTOR
            .may_load(&storage, scope.clone())
            .unwrap()
            .is_none());
    }
}
