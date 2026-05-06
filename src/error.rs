use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("Standard error: {0}")]
    Std(#[from] StdError),

    /// Occurs when there is an error querying a balance
    #[error("Failed to query balance")]
    BalanceQueryError,

    /// Occurs when an action must be performed by the contributor but is attempted by a different address
    #[error("Marker assets have changed since initiation of contribution")]
    InconsistentMarkerState,

    /// Occurs when a marker has multiple holders of the marker coins
    #[error("Multiple holders of marker coins detected, only single ownership allowed")]
    MultipleMarkerHolders,

    /// Occurs when the contributor is not the owner of the marker coins
    #[error("Unauthorized action attempted by non-contributor address")]
    UnauthorizedContributorAction,

    /// Occurs when an empty scope list is passed by the OTC
    #[error("Empty list of scopes is not allowed")]
    NoScopesProvided,

    /// Occurs when an unauthorized address attempts to perform a restricted action
    #[error("Unauthorized action attempted by non-OTC address")]
    UnauthorizedOTC,

    /// Occurs when the list of balances is greater than one
    #[error("Marker holding has multiple balances, expected single balance")]
    MultipleMarkerBalances,

    /// Occurs when unable to query marker information
    #[error("Failed to query marker information: {0}")]
    MarkerQueryError(String),

    /// Occurs when unable to query scope information
    #[error("Failed to query scope information: {0}")]
    ScopeQueryError(String),

    /// Occurs when no funds are sent during redemption finalization
    #[error("Invalid funds sent during redemption finalization")]
    InvalidFundsSent,

    /// Occurs when an invalid coin is sent during redemption finalization
    #[error("Invalid coin sent during redemption finalization")]
    InvalidCoinSent,

    /// Occurs when an amount of ldt tokens is sent that doesn't match the expected amount
    #[error("Sent ldt token amount for redemption does not match expected amount")]
    IncorrectRedemptionAmount,

    /// Occurs when redemption pooling is not yet complete
    #[error("Redemption pooling is not yet complete")]
    RedemptionPoolingIncomplete,

    /// Occurs when an invalid number of coins is specified for redemption initiation
    #[error("Invalid number of coins specified for redemption initiation")]
    InvalidNumberOfCoins,

    /// Occurs when an invalid burn amount is specified for redemption initiation
    #[error("Invalid burn amount specified for redemption initiation")]
    InvalidBurnAmount,

    /// Occurs when a scope UUID is not in the correct format
    #[error("Invalid scope UUID format: {0}")]
    InvalidScopeUuidFormat(String),

    /// Occurs when a list of scopes provided by the OTC contains duplicate scope UUIDs
    #[error("Duplicate scope UUIDs detected in provided scope list")]
    DuplicateScopeUuids,

    /// Occurs when a contribution with the specified ID is not found in storage
    #[error("Contribution with ID {contribution_id} not found")]
    ContributionNotFound { contribution_id: u64 },

    /// Occurs when a contribution reference is not found in storage
    #[error("Contribution reference {contribution_ref} not found")]
    ContributionReferenceNotFound { contribution_ref: String },

    /// Occurs when a redemption reference is not found in storage
    #[error("Redemption reference {redemption_ref} not found")]
    RedemptionReferenceNotFound { redemption_ref: String },

    /// Occurs when a redemption with the specified ID is not found in storage
    #[error("Redemption with ID {redemption_id} not found")]
    RedemptionNotFound { redemption_id: u64 },

    /// Occurs when a scope is provided for contribution but that scope is already part of a pending contribution
    #[error("Scope provided is already part of a pending contribution")]
    ScopeAlreadyInPendingContribution,

    /// Occurs when finalizing a contribution for a scope that was already attributed to a contributor
    #[error("Scope {scope_uuid} was already contributed and attributed")]
    ScopeAlreadyContributed { scope_uuid: String },

    /// Occurs when a scope is provided for redemption but that scope is already part of a pending redemption
    #[error("Scope provided is already part of a pending redemption")]
    ScopeAlreadyInPendingRedemption,

    /// Occurs when a reference string provided is too long
    #[error("Reference string is too long, maximum length is 128 characters")]
    ReferenceTooLong,

    /// Occurs when optional contract app metadata exceeds allowed length for `kind` or `data`
    #[error("Contract app metadata field `{field}` exceeds maximum allowed length")]
    ContractAppMetadataTooLong { field: &'static str },

    /// Occurs when a reference provided already exists in storage
    #[error("Reference {reference} already exists")]
    ReferenceAlreadyExists { reference: String },

    /// Occurs when an unauthorized address attempts to finalize a redemption
    #[error("Unauthorized address attempted to finalize redemption")]
    UnauthorizedRedeemer,

    /// Occurs when an action must be performed by the CosmWasm contract admin but is attempted by a different address (or the contract has no admin set)
    #[error("Unauthorized action attempted by non-contract-admin address")]
    UnauthorizedContractAdmin,

    /// Occurs when a scope provided by the OTC for redemption is not owned by the contract
    #[error("Unauthorized scope provided for redemption, contract does not own the scope")]
    UnauthorizedScopeForRedemption,

    /// Occurs when orphan-marker cleanup is invoked with an empty denom list
    #[error("Empty list of marker denoms is not allowed for orphan cleanup")]
    EmptyOrphanMarkerDenoms,

    /// Occurs when the contract does not hold a positive bank balance of the marker denom
    #[error("Contract does not hold marker coins for denom: {denom}")]
    OrphanMarkerNotHeldByContract { denom: String },

    /// Occurs when the marker still value-owns at least one metadata scope
    #[error("Marker {denom} still value-owns metadata scopes")]
    OrphanMarkerStillOwnsScopes { denom: String },

    /// Occurs when orphan cleanup targets the configured LDT denom
    #[error("Cannot cleanup the LDT denom via orphan marker cleanup")]
    CleanupLdtDenomNotAllowed,

    /// Occurs when a pending redemption still references this marker as its pool denom
    #[error("Cannot cleanup denom {denom}: pending redemption {redemption_id} uses it as pool_denom")]
    OrphanMarkerReferencedByPendingRedemption { denom: String, redemption_id: u64 },
}
