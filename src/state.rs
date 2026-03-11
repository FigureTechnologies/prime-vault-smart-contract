use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw2::ContractVersion;
use cw_storage_plus::{Item, Map};

// version info for migration info
pub const CONTRACT_NAME: &str = "crates.io:loan-dicer";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    pub trust_address: Addr,
    pub ldt_denom: String,
    pub redemption_marker_admin: Addr,
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
