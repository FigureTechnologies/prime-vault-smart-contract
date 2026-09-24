use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw2::ContractVersion;
use cw_storage_plus::{Item, Map};

use crate::error::ContractError;

// version info for migration info
pub const CONTRACT_NAME: &str = "crates.io:loan-dicer";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MAX_CONTRACT_APP_METADATA_KIND_LEN: usize = 64;
pub const MAX_CONTRACT_APP_METADATA_DATA_LEN: usize = 1024;
pub const MAX_CONTRIBUTION_REF_LEN: usize = 128;
pub const MAX_REDEMPTION_REF_LEN: usize = 128;
pub const MAX_SWAP_REF_LEN: usize = 128;

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
pub struct StoredAccessGrant {
    pub address: String,
    pub permissions: Vec<i32>,
}

#[cw_serde]
pub struct PendingContribution {
    pub contributor_addr: Addr,
    pub contribution_ref: Option<String>,
    pub marker_denom: Option<String>,
    /// LDT the OTC offers for the escrowed marker, set by `PriceContribution` after submit.
    /// Re-pricing is allowed because the contributor has committed nothing on the strength of
    /// this figure; `ConfirmContribution` pins the value the contributor actually agreed to.
    #[serde(default)]
    pub mint_ldt_amount: Option<u64>,
    /// Access grants removed from the marker when the contributor submitted.
    #[serde(default)]
    pub stored_access_grants: Option<Vec<StoredAccessGrant>>,
}

#[cw_serde]
pub struct PendingSwap {
    pub pool_denom: String,
    pub pooling_complete: bool,
    pub contributor_addr: Addr,
    pub swap_ref: Option<String>,
    /// LDT to mint to the contributor on confirm. Mutually exclusive with `burn_ldt_amount`.
    #[serde(default)]
    pub mint_ldt_amount: u64,
    /// LDT the contributor must send on submit and that is burned on confirm.
    /// Mutually exclusive with `mint_ldt_amount`.
    #[serde(default)]
    pub burn_ldt_amount: u64,
    /// Incoming marker denom once the contributor has submitted swap assets.
    #[serde(default)]
    pub incoming_marker_denom: Option<String>,
    /// Access grants removed from the incoming marker when the contributor submitted.
    #[serde(default)]
    pub stored_incoming_access_grants: Option<Vec<StoredAccessGrant>>,
}

#[cw_serde]
pub struct PendingRedemption {
    pub burn_ldt_amount: Option<u64>,
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
pub const PENDING_SWAPS: Map<u64, PendingSwap> = Map::new("swaps");
pub const CONTRIBUTION_COUNTER: Item<u64> = Item::new("contribution_counter");
pub const REDEMPTION_COUNTER: Item<u64> = Item::new("redemption_counter");
pub const SWAP_COUNTER: Item<u64> = Item::new("swap_counter");
pub const CONFIGURATION: Item<Configuration> = Item::new("configuration");
pub const CONTRIBUTION_REFERENCES: Map<String, u64> = Map::new("contribution_references");
pub const REDEMPTION_REFERENCES: Map<String, u64> = Map::new("redemption_references");
pub const SWAP_REFERENCES: Map<String, u64> = Map::new("swap_references");
/// Reverse index of in-flight marker denoms → the pending record that owns them.
/// Swap `pool_denom` and `incoming_marker_denom`, redemption `pool_denom`, and
/// contribution `marker_denom` each get an entry for O(1) pooling/cleanup checks.
pub const PENDING_DENOMS: Map<String, PendingDenomOwner> = Map::new("pending_denoms");

/// Which pending record currently claims an in-flight marker denom.
#[cw_serde]
#[derive(Copy, Eq)]
pub enum PendingDenomOwner {
    Swap { id: u64 },
    Redemption { id: u64 },
    Contribution { id: u64 },
}
