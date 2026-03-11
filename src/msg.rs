use cosmwasm_schema::QueryResponses;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::Configuration;

/// The msg that is sent to the chain in order to instantiate a new instance of this contract's
/// stored code. Used in the functionality defined in [instantiate_contract](crate::instantiate::instantiate_contract::instantiate_contract).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateContractMsg {
    /// The address of the OTC that will interact with the contract
    pub otc_address: String,
    /// The address of the trust for the contract
    pub trust_address: String,
    /// The denom of the loan dicer tokens to mint
    pub ldt_denom: String,
    /// The address that will be the admin of redemption markers created during redemption
    pub redemption_marker_admin: String,
}

/// All defined payloads to be used when executing routes on this contract instance.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum ExecuteMsg {
    /// A route that initiates a contribution of assets from a contributor. This will create a
    /// pending contribution (no tokens will be minted yet).
    InitiateContribution {
        /// The ids of the scopes to be contributed
        scope_uuids: Vec<String>,
        /// The number of loan dicer tokens to mint
        mint_ldt_amount: u64,
        /// A reference String that can be used to look up the contribution. This is not used by the
        /// contract but can be helpful for off-chain indexing and tracking of contributions.
        contribution_ref: Option<String>,
    },
    /// A route that finalizes a contribution of assets from a contributor. This will mint tokens
    /// and transfer them to the contributor.
    FinalizeContribution {
        /// The ID of the contribution to be finalized
        contribution_id: u64,
        /// Denom of marker whose scopes are being contributed
        marker_denom: String,
    },
    /// A route that cancels a pending contribution. This can only be called by the OTC and will remove the pending contribution without minting any tokens.
    CancelContribution {
        /// The ID of the contribution to be canceled
        contribution_id: u64,
    },
    /// A route used by the token holder that redeems loan dicer tokens for underlying assets.
    /// This will create a pending redemption (no assets will be transferred yet).
    InitiateRedemption {
        /// The number of loan dicer tokens to burn
        burn_ldt_amount: u64,
        /// The ids of the scopes to be redeemed
        scope_uuids: Vec<String>,
        /// Marker to pool the scopes under
        pool_denom: String,
        /// Number of marker coins to create in the pooling process
        num_coins: u64,
        /// The address that is allowed to finalize the redemption
        redeemer_address: String,
        /// A reference String that can be used to look up the redemption. This is not used by the
        /// contract but can be helpful for off-chain indexing and tracking of redemptions.
        redemption_ref: Option<String>,
    },
    PoolRedemption {
        /// The ID of the redemption to be pooled
        redemption_id: u64,
        /// The number of scopes to pool
        num_scopes_to_pool: u32,
    },
    /// A route that finalizes a redemption of loan dicer tokens. This will transfer the
    /// underlying assets to the redeemer.
    FinalizeRedemption {
        /// The ID of the redemption to be finalized
        redemption_id: u64,
    },
    // A route that cancels a pending redemption. This can only be called by the OTC and will remove the pending redemption without transferring any assets.
    CancelRedemption {
        /// The ID of the redemption to be canceled
        redemption_id: u64,
    },
}
/// All defined payloads to be used when querying routes on this contract instance.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, QueryResponses)]
pub enum QueryMsg {
    /// A route to query the details of a pending contribution by its ID.
    #[returns(GetContributionResponse)]
    GetPendingContribution { contribution_id: u64 },

    /// A route to query the details of a pending contribution by its reference string.
    #[returns(GetContributionResponse)]
    GetPendingContributionByReference { contribution_ref: String },

    /// A route to query the details of a pending redemption by its ID.
    #[returns(GetRedemptionResponse)]
    GetPendingRedemption { redemption_id: u64 },

    /// A route to query the details of a pending redemption by its reference string.
    #[returns(GetRedemptionResponse)]
    GetPendingRedemptionByReference { redemption_ref: String },

    /// A route to query the contract configuration details.
    #[returns(Configuration)]
    GetConfiguration {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetContributionResponse {
    pub contribution_id: u64,
    pub mint_ldt_amount: u64,
    pub scope_uuids: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetRedemptionResponse {
    pub redemption_id: u64,
    pub burn_ldt_amount: u64,
    pub scope_uuids: Vec<String>,
    pub pool_denom: String,
    pub pooling_complete: bool,
    pub redeemer_addr: String,
}
