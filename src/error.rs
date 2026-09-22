use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("Standard error: {0}")]
    Std(#[from] StdError),

    /// Occurs when there is an error querying a balance
    #[error("Failed to query balance")]
    BalanceQueryError,

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

    /// Occurs when a holding query resolves to a different denom than the marker query
    #[error(
        "Resolved marker holding denom {holding_denom} does not match marker denom {marker_denom}"
    )]
    MarkerDenomMismatch {
        holding_denom: String,
        marker_denom: String,
    },

    /// Occurs when a marker denom parses as a bech32 account address. Marker queries try
    /// address first, so such a denom can resolve a different marker than denom-only messages.
    #[error(
        "Marker denom {denom} is a valid account address and cannot be escrowed or looked up by denom"
    )]
    MarkerDenomIsAddress { denom: String },

    /// Occurs when a denom-only marker/holding query resolved a different marker than requested
    #[error(
        "Marker query for denom {requested_denom} resolved a different marker ({resolved_denom})"
    )]
    MarkerDenomResolutionMismatch {
        requested_denom: String,
        resolved_denom: String,
    },

    /// Occurs when the contract does not hold the marker permissions required to take or keep custody
    #[error("Contract does not hold required marker access (Admin, Withdraw, Deposit, Transfer)")]
    InsufficientMarkerAccess,

    /// Occurs when a submitted marker is not Active; TransferCoin would fail at the keeper
    #[error("Marker must be Active to escrow, found status {status}")]
    MarkerNotActive { status: i32 },

    /// Occurs when a submitted marker is not Restricted; TransferCoin would fail at the keeper
    #[error("Marker must be Restricted type to escrow, found type {marker_type}")]
    MarkerNotRestricted { marker_type: i32 },

    /// Occurs when UpdateConfiguration is called without naming any field to change
    #[error("UpdateConfiguration requires at least one field to update")]
    EmptyConfigurationUpdate,

    /// Occurs when a submit route is handed the contract's own LDT marker. Escrowing it would
    /// strip the other access grants, and a later cancel would delete the contract's own grant,
    /// destroying the contract's Mint, Burn, and Admin authority over LDT.
    #[error("The LDT marker {denom} cannot be escrowed")]
    CannotEscrowLdtMarker { denom: String },

    /// Occurs when unable to query scope information
    #[error("Failed to query scope information: {0}")]
    ScopeQueryError(String),

    /// Occurs when attached funds are missing, unexpected, or otherwise invalid
    #[error("Invalid funds sent")]
    InvalidFundsSent,

    /// Occurs when an invalid coin is sent during redemption finalization
    #[error("Invalid coin sent during redemption finalization")]
    InvalidCoinSent,

    /// Occurs when an amount of ldt tokens is sent that doesn't match the expected amount
    #[error("Sent ldt token amount does not match expected amount")]
    IncorrectTokenAmount,

    /// Occurs when redemption pooling is not yet complete
    #[error("Redemption pooling is not yet complete")]
    RedemptionPoolingIncomplete,

    /// Occurs when PoolRedemption is called after pooling has already been marked complete
    #[error("Redemption pooling is already complete")]
    RedemptionPoolingAlreadyComplete,

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

    /// Occurs when a contribution denom has not been set
    #[error("Contribution denom has not been set")]
    ContributionDenomNotSet,

    /// Occurs when SubmitContribution is called after assets have already been submitted
    #[error("Contribution denom has already been set")]
    ContributionDenomAlreadySet,

    /// Occurs when ConfirmContribution is called before the OTC has priced the contribution
    #[error("Contribution has not been priced by the OTC")]
    ContributionNotPriced,

    /// Occurs when the price the contributor confirmed no longer matches the stored offer,
    /// which means the OTC re-priced the contribution before the confirm landed
    #[error("Confirmed mint amount {expected} does not match the current offer of {actual}")]
    ContributionPriceMismatch { expected: u64, actual: u64 },

    /// Occurs when an unauthorized address attempts to cancel a contribution
    #[error("Unauthorized action attempted to cancel contribution")]
    UnauthorizedContributionCancel,

    /// Occurs when a redemption reference is not found in storage
    #[error("Redemption reference {redemption_ref} not found")]
    RedemptionReferenceNotFound { redemption_ref: String },

    /// Occurs when a redemption with the specified ID is not found in storage
    #[error("Redemption with ID {redemption_id} not found")]
    RedemptionNotFound { redemption_id: u64 },

    /// Occurs when a reference string provided is too long
    #[error("Reference string is too long, maximum length is 128 characters")]
    ReferenceTooLong,

    /// Occurs when a reference string is supplied but is empty or only whitespace. Storing one
    /// would claim the empty key in the reference map and block every later record from using it.
    #[error("Reference string cannot be empty or whitespace-only")]
    ReferenceEmpty,

    /// Occurs when optional contract app metadata exceeds allowed length for `kind` or `data`
    #[error("Contract app metadata field `{field}` exceeds maximum allowed length")]
    ContractAppMetadataTooLong { field: &'static str },

    /// Occurs when a reference provided already exists in storage
    #[error("Reference {reference} already exists")]
    ReferenceAlreadyExists { reference: String },

    /// Occurs when an unauthorized address attempts to confirm a redemption
    #[error("Unauthorized address attempted to confirm redemption")]
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
    #[error(
        "Cannot cleanup denom {denom}: pending redemption {redemption_id} uses it as pool_denom"
    )]
    OrphanMarkerReferencedByPendingRedemption { denom: String, redemption_id: u64 },

    /// Occurs when a pending contribution still references this marker as its marker denom
    #[error(
        "Cannot cleanup denom {denom}: pending contribution {contribution_id} uses it as marker_denom"
    )]
    OrphanMarkerReferencedByPendingContribution { denom: String, contribution_id: u64 },

    /// Occurs when migrate is invoked on storage that does not match this contract's cw2 name
    #[error("Cannot migrate: expected contract name {expected}, found {found}")]
    CannotMigrateContract { expected: String, found: String },

    /// Occurs when migrate is invoked from a stored version this code does not support
    #[error("Cannot migrate from contract version {version}")]
    CannotMigrateFromVersion { version: String },

    /// Occurs when migrate is invoked while pending contributions, redemptions, or swaps exist
    #[error(
        "Cannot migrate while pending records exist: {contributions} contributions, {redemptions} redemptions, {swaps} swaps"
    )]
    PendingRecordsExist {
        contributions: u32,
        redemptions: u32,
        swaps: u32,
    },

    /// Occurs when both mint and burn LDT amounts are specified for a swap
    #[error("Swap cannot specify both mint and burn LDT amounts")]
    ConflictingSwapTokenAmounts,

    /// Occurs when a swap with the specified ID is not found in storage
    #[error("Swap with ID {swap_id} not found")]
    SwapNotFound { swap_id: u64 },

    /// Occurs when a swap reference is not found in storage
    #[error("Swap reference {swap_ref} not found")]
    SwapReferenceNotFound { swap_ref: String },

    /// Occurs when swap pooling is not yet complete
    #[error("Swap pooling is not yet complete")]
    SwapPoolingIncomplete,

    /// Occurs when PoolSwap is called after pooling has already been marked complete
    #[error("Swap pooling is already complete")]
    SwapPoolingAlreadyComplete,

    /// Occurs when ConfirmSwap is called before the contributor has submitted an incoming marker
    #[error("Swap incoming marker denom has not been set")]
    SwapIncomingDenomNotSet,

    /// Occurs when SubmitSwap is called after incoming assets have already been submitted
    #[error("Swap incoming marker denom has already been set")]
    SwapIncomingDenomAlreadySet,

    /// Occurs when an unauthorized address attempts to submit a swap
    #[error("Unauthorized address attempted to submit swap")]
    UnauthorizedSwapContributor,

    /// Occurs when an unauthorized address attempts to cancel a swap
    #[error("Unauthorized action attempted to cancel swap")]
    UnauthorizedSwapCancel,

    /// Occurs when a pending swap still references this marker as its pool denom or incoming marker denom
    #[error(
        "Cannot cleanup denom {denom}: pending swap {swap_id} uses it as pool_denom or incoming_marker_denom"
    )]
    OrphanMarkerReferencedByPendingSwap { denom: String, swap_id: u64 },

    /// Occurs when a scope provided by the OTC for swap pooling is not owned by the contract
    #[error("Unauthorized scope provided for swap, contract does not own the scope")]
    UnauthorizedScopeForSwap,

    /// Occurs when a scope's value-owner marker is the pool or incoming marker of another live pending swap
    #[error(
        "Cannot pool scope from denom {denom}: pending swap {swap_id} uses it as pool_denom or incoming_marker_denom"
    )]
    ScopeOwnedByPendingSwap { denom: String, swap_id: u64 },

    /// Occurs when a scope's value-owner marker is the pool marker of a live pending redemption
    #[error(
        "Cannot pool scope from denom {denom}: pending redemption {redemption_id} uses it as pool_denom"
    )]
    ScopeOwnedByPendingRedemption { denom: String, redemption_id: u64 },

    /// Occurs when a scope's value-owner marker is the submitted marker of a live pending contribution
    #[error(
        "Cannot pool scope from denom {denom}: pending contribution {contribution_id} uses it as marker_denom"
    )]
    ScopeOwnedByPendingContribution { denom: String, contribution_id: u64 },

    /// Occurs when a marker denom is already claimed by another in-flight pending record
    #[error("Marker denom {denom} is already in use by a pending record")]
    PendingDenomAlreadyInUse { denom: String },

    /// Occurs when the burn amount is not specified for a redemption
    #[error("Missing burn amount for redemption")]
    MissingBurnAmount,
}
