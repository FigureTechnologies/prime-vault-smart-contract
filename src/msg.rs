use cosmwasm_schema::QueryResponses;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::ContractAppMetadata;

/// Message sent when upgrading this contract instance to new code via `MsgMigrateContract`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct MigrateMsg {}

/// The msg that is sent to the chain in order to instantiate a new instance of this contract's
/// stored code. Used in the functionality defined in [instantiate_contract](crate::instantiate::instantiate_contract::instantiate_contract).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateContractMsg {
    /// The address of the OTC that will interact with the contract
    pub otc_address: String,
    /// The denom of the loan dicer tokens to mint
    pub ldt_denom: String,
    /// The address that will be the admin of redemption markers created during redemption
    pub redemption_marker_admin: String,
    /// Optional app-defined metadata for this contract instance; not interpreted semantically by the contract.
    #[serde(default)]
    pub app_metadata: Option<ContractAppMetadata>,
}

/// All defined payloads to be used when executing routes on this contract instance.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum ExecuteMsg {
    /// A route that initiates a contribution of assets from a contributor. This will create a
    /// pending contribution (no tokens will be minted yet).
    InitiateContribution {
        /// The address of the contributor that must provide the assets for the contribution
        contributor_address: String,
        /// A reference String that can be used to look up the contribution. This is not used by the
        /// contract but can be helpful for off-chain indexing and tracking of contributions.
        contribution_ref: Option<String>,
    },
    /// A route where the contributor escrows a restricted marker for OTC review.
    /// The contract must already have Admin on the marker. Before calling, the
    /// contributor must grant this contract `MarkerTransferAuthorization` (authz)
    /// covering the full marker holding; the contract then transfers those coins
    /// from the contributor into the marker account. Without the grant, the marker
    /// keeper rejects the transfer.
    SubmitContribution {
        /// The ID of the contribution to be submitted
        contribution_id: u64,
        /// The denom of the marker being used to submit the assets
        marker_denom: String,
    },
    /// A route where the OTC offers a price, in LDT, for a marker the contributor has already
    /// escrowed. Nothing is minted or moved here — the offer only becomes binding when the
    /// contributor calls `ConfirmContribution`.
    ///
    /// Re-pricing an already-priced contribution is allowed: the contributor has committed
    /// nothing on the strength of the previous figure, so correcting a mistake does not require
    /// unwinding the escrow. `ConfirmContribution` carries the amount the contributor agreed to,
    /// so a re-price cannot be used to settle at a figure they never saw.
    PriceContribution {
        /// The ID of the contribution to be priced. Must already have a submitted marker.
        contribution_id: u64,
        /// The amount of loan dicer tokens offered for the escrowed marker
        mint_ldt_amount: u64,
    },
    /// A route where the contributor confirms the OTC's price and settles the contribution. This
    /// mints the offered LDT to the contributor and takes the escrowed marker into contract
    /// custody. Only the contributor may call it, and only after `PriceContribution`.
    ConfirmContribution {
        /// The ID of the contribution to be confirmed
        contribution_id: u64,
        /// The price the contributor is confirming. Must equal the currently stored
        /// `mint_ldt_amount`, so a concurrent re-price cannot settle at a different figure.
        expected_mint_ldt_amount: u64,
    },
    /// A route that cancels a pending contribution. This can be called by the OTC or the
    /// contributor. If the contributor has already submitted assets, marker access is restored
    /// and marker coins are returned to the contributor.
    CancelContribution {
        /// The ID of the contribution to be canceled
        contribution_id: u64,
    },
    /// A route used by the token holder that redeems loan dicer tokens for underlying assets.
    /// This will create a pending redemption (no assets will be transferred yet).
    InitiateRedemption {
        pool_denom: String,
        /// Number of marker coins to create in the pooling process
        num_coins: u64,
        /// The address that is allowed to confirm the redemption
        redeemer_address: String,
        /// A reference String that can be used to look up the redemption. This is not used by the
        /// contract but can be helpful for off-chain indexing and tracking of redemptions.
        redemption_ref: Option<String>,
    },
    PoolRedemption {
        /// The ID of the redemption to be pooled
        redemption_id: u64,
        /// A list of scope IDs to add to the redemption pool. Rejected after pooling is marked
        /// complete, and rejected when a scope is value-owned by another live pending swap or
        /// redemption pool.
        scope_uuids: Vec<String>,
    },
    /// A route that marks redemption pooling complete and sets the number of LDT tokens to burn.
    /// This can only be called by the OTC after all scopes have been pooled.
    CompleteRedemptionPool {
        /// The ID of the redemption whose pooling is complete
        redemption_id: u64,
        /// The number of loan dicer tokens the redeemer must burn upon confirm
        burn_ldt_amount: u64,
    },
    /// A route that confirms a redemption of loan dicer tokens. This will transfer the
    /// underlying assets to the redeemer.
    ConfirmRedemption {
        /// The ID of the redemption to be confirmed
        redemption_id: u64,
    },
    /// A route that cancels a pending redemption. This can only be called by the OTC and will remove the pending redemption without transferring any assets.
    CancelRedemption {
        /// The ID of the redemption to be canceled
        redemption_id: u64,
    },
    /// Admin-only maintenance: delete specific marker denoms the contract holds in its bank balance
    /// that no longer value-own any metadata scopes and are not referenced by any pending contribution
    /// (`marker_denom`), redemption (`pool_denom`), or swap (`pool_denom` / `incoming_marker_denom`).
    /// Duplicate denoms are de-duplicated; processing order is ascending lexicographic denom. The
    /// entire message fails if any denom is invalid.
    CleanupOrphanMarkers {
        /// Marker denoms to delete (must be non-empty). Each must be a marker the contract holds with
        /// a positive balance, must not be the configured LDT denom, must not value-own any scope, and
        /// must not be referenced by a pending contribution, redemption, or swap.
        marker_denoms: Vec<String>,
    },
    /// Admin-only: rotate the privileged addresses in configuration. Omitted fields are left
    /// unchanged, and at least one must be supplied.
    ///
    /// Without this route a lost or compromised `otc_address` would strand every contract-held
    /// asset, since the migrate escape hatch is blocked while any pending record exists.
    ///
    /// `ldt_denom` is deliberately not rotatable: it is bound to the marker created at
    /// instantiate, so changing it would orphan that marker along with the contract's mint and
    /// burn authority.
    UpdateConfiguration {
        /// New OTC address. Takes effect immediately for every privileged route, including
        /// records that are already in flight.
        otc_address: Option<String>,
        /// New redemption marker admin. Applies only to redemptions and swaps confirmed after
        /// this call; pool markers already settled granted custody to the previous admin and
        /// are recoverable through governance rather than by rotating here.
        redemption_marker_admin: Option<String>,
    },
    /// A route that initiates a swap of contract-held scopes for contributor-provided scopes.
    /// This will create a pending swap (no assets will be transferred yet).
    InitiateSwap {
        /// Marker to pool the removed scopes under
        pool_denom: String,
        /// Number of marker coins to create in the pooling process
        num_coins: u64,
        /// The address that is allowed to submit incoming scopes for the swap
        contributor_address: String,
        /// A reference String that can be used to look up the swap. This is not used by the
        /// contract but can be helpful for off-chain indexing and tracking of swaps.
        swap_ref: Option<String>,
    },
    /// Move removed scopes from contract-held markers onto the swap pool marker.
    /// Rejected after pooling is marked complete or after the contributor has submitted,
    /// and rejected when a scope is value-owned by another live pending swap or redemption pool.
    PoolSwap {
        /// The ID of the swap to be pooled
        swap_id: u64,
        /// Scopes to pool onto the swap pool marker
        removed_scope_uuids: Vec<String>,
    },
    /// A route that marks swap pooling complete and sets the number of LDT tokens to mint or burn.
    /// This can only be called by the OTC after all removed scopes have been pooled.
    ///
    /// Terms are write-once: the call is rejected once pooling is already complete, so the
    /// amounts a contributor sees before submitting cannot change afterwards. Correcting a
    /// mistake means cancelling the swap and initiating a new one with a fresh pool denom.
    CompleteSwapPool {
        /// The ID of the swap whose pooling is complete
        swap_id: u64,
        /// The amount of loan dicer tokens to mint to the contributor (mutually exclusive with burn)
        mint_ldt_amount: u64,
        /// The amount of loan dicer tokens the contributor must send on submit (mutually exclusive with mint)
        burn_ldt_amount: u64,
    },
    /// A route where the contributor submits incoming scopes via a restricted marker
    /// for OTC review. The contract must already have Admin on the incoming marker.
    /// Before calling, the contributor must grant this contract
    /// `MarkerTransferAuthorization` (authz) covering the full incoming marker
    /// holding; the contract then transfers those coins from the contributor into
    /// the marker account. Without the grant, the marker keeper rejects the
    /// transfer. If the swap has a burn amount, the contributor must also attach
    /// that LDT with this message.
    SubmitSwap {
        /// The ID of the swap whose incoming scopes are being submitted
        swap_id: u64,
        /// Denom of marker whose scopes are being contributed as incoming scopes
        incoming_marker_denom: String,
    },
    /// A route that confirms a swap after the contributor has submitted assets. The OTC mints
    /// LDT if configured, burns LDT already paid by the contributor, withdraws the incoming
    /// marker into contract custody, and transfers the pooled scopes to the contributor.
    ConfirmSwap {
        /// The ID of the swap to be confirmed
        swap_id: u64,
    },
    /// A route that cancels a pending swap. This can be called by the OTC or the contributor.
    /// If the contributor has already submitted assets, marker access is restored and marker coins
    /// are returned to the contributor.
    CancelSwap {
        /// The ID of the swap to be canceled
        swap_id: u64,
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

    /// A route to query the details of a pending swap by its ID.
    #[returns(GetSwapResponse)]
    GetPendingSwap { swap_id: u64 },

    /// A route to query the details of a pending swap by its reference string.
    #[returns(GetSwapResponse)]
    GetPendingSwapByReference { swap_ref: String },

    /// A route to query the contract configuration details.
    #[returns(crate::state::Configuration)]
    GetConfiguration {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetContributionResponse {
    pub contribution_id: u64,
    pub contributor_addr: String,
    pub marker_denom: Option<String>,
    /// The OTC's current LDT offer, once `PriceContribution` has been called. The contributor
    /// reads this to decide whether to confirm, and echoes it back in `ConfirmContribution`.
    pub mint_ldt_amount: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetRedemptionResponse {
    pub redemption_id: u64,
    pub burn_ldt_amount: Option<u64>,
    pub pool_denom: String,
    pub pooling_complete: bool,
    pub redeemer_addr: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GetSwapResponse {
    pub swap_id: u64,
    pub pool_denom: String,
    pub pooling_complete: bool,
    pub contributor_addr: String,
    pub mint_ldt_amount: u64,
    pub burn_ldt_amount: u64,
    /// Incoming marker denom once the contributor has submitted. `ConfirmSwap` requires this
    /// alongside `pooling_complete`; `None` means submit has not happened yet.
    pub incoming_marker_denom: Option<String>,
}
