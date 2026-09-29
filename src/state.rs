use std::collections::HashSet;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};
use cw2::ContractVersion;
use cw_storage_plus::{Item, Map};
use uuid::Uuid;

use crate::error::ContractError;

// version info for migration info
pub const CONTRACT_NAME: &str = "crates.io:loan-dicer";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MAX_CONTRACT_APP_METADATA_KIND_LEN: usize = 64;
pub const MAX_CONTRACT_APP_METADATA_DATA_LEN: usize = 1024;
pub const MAX_CONTRIBUTION_REF_LEN: usize = 128;
pub const MAX_REDEMPTION_REF_LEN: usize = 128;
pub const MAX_SWAP_REF_LEN: usize = 128;
pub const MAX_SETTLEMENT_REF_LEN: usize = 128;

/// App-defined metadata for this contract instance (not interpreted by the contract).
#[cw_serde]
pub struct ContractAppMetadata {
    /// Optional discriminator, e.g. external transaction flow type.
    pub kind: Option<String>,
    /// Opaque payload (convention: JSON string).
    pub data: Option<String>,
}

/// Marker type used when creating the loan dicer token denom.
///
/// This value is immutable in the Provenance marker module, so it must be
/// selected correctly when the contract is instantiated.
#[cw_serde]
#[derive(Copy, Eq)]
pub enum LdtMarkerType {
    Coin,
    Restricted,
}

impl Default for LdtMarkerType {
    fn default() -> Self {
        Self::Coin
    }
}

/// Redemption and swap burns arrive as a bank send from the holder. A restricted PI marker
/// rejects that send, so those routes are refused instead of creating a record that cannot settle.
pub fn ensure_holder_funded_pi_burn(marker_type: LdtMarkerType) -> Result<(), ContractError> {
    if marker_type == LdtMarkerType::Restricted {
        return Err(ContractError::RestrictedMarkerHolderBurnNotSupported);
    }
    Ok(())
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

/// Forced transfer and required attributes are restricted-marker features. Attribute names
/// are normalized (lowercase, trimmed segments) and must be unique, matching Provenance name
/// records. A leading `*.` wildcard is allowed; `*` in any other position is not.
pub fn validate_ldt_marker_policy(
    marker_type: LdtMarkerType,
    allow_forced_transfer: bool,
    required_attributes: &[String],
) -> Result<Vec<String>, ContractError> {
    if marker_type != LdtMarkerType::Restricted {
        if allow_forced_transfer {
            return Err(ContractError::InvalidLdtMarkerPolicy(
                "allow_forced_transfer can only be true when marker_type is restricted".to_string(),
            ));
        }
        if !required_attributes.is_empty() {
            return Err(ContractError::InvalidLdtMarkerPolicy(
                "required_attributes can only be set when marker_type is restricted".to_string(),
            ));
        }
        return Ok(vec![]);
    }

    let mut normalized = Vec::with_capacity(required_attributes.len());
    let mut seen = HashSet::new();
    for attribute in required_attributes {
        let normalized_attribute = normalize_and_validate_required_attribute(attribute)?;
        if !seen.insert(normalized_attribute.clone()) {
            return Err(ContractError::InvalidLdtMarkerPolicy(format!(
                "duplicate required attribute: {normalized_attribute}"
            )));
        }
        normalized.push(normalized_attribute);
    }
    Ok(normalized)
}

fn normalize_and_validate_required_attribute(attribute: &str) -> Result<String, ContractError> {
    let invalid = || {
        ContractError::InvalidLdtMarkerPolicy(format!(
            "invalid required attribute name: {attribute}"
        ))
    };

    if attribute.trim().is_empty() {
        return Err(invalid());
    }

    let segments: Vec<String> = attribute
        .split('.')
        .map(|segment| segment.trim().to_lowercase())
        .collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(invalid());
    }

    for (index, segment) in segments.iter().enumerate() {
        if segment == "*" {
            if index != 0 || segments.len() < 2 {
                return Err(invalid());
            }
            continue;
        }
        if !is_valid_name_segment(segment) {
            return Err(invalid());
        }
    }

    Ok(segments.join("."))
}

fn is_valid_name_segment(segment: &str) -> bool {
    if Uuid::parse_str(segment).is_ok() {
        return true;
    }
    if segment.chars().filter(|c| *c == '-').count() > 1 {
        return false;
    }
    segment
        .chars()
        .all(|c| c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit())
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
    /// Immutable type of the LDT marker created during instantiation.
    #[serde(default)]
    pub marker_type: LdtMarkerType,
    /// Whether the LDT marker permits an authorized address to force-transfer tokens.
    #[serde(default)]
    pub allow_forced_transfer: bool,
    /// Attributes an address must hold to transfer LDT without marker transfer access.
    #[serde(default)]
    pub required_attributes: Vec<String>,
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
/// Completed collection settlements, keyed by the external settlement reference.
/// Presence is the idempotency record. Holder addresses are not stored.
pub const SETTLEMENTS: Map<String, CompletedSettlement> = Map::new("settlements");

/// Outcome of a completed `SettleCollection`, returned by `GetSettlement`.
#[cw_serde]
pub struct CompletedSettlement {
    pub total_pi_burned: Uint128,
    pub holder_count: u32,
}

/// Trim a settlement reference so `"run-1"` and `" run-1 "` are the same idempotency key.
pub fn normalize_settlement_ref(settlement_ref: &str) -> Result<String, ContractError> {
    let settlement_ref = settlement_ref.trim();
    if settlement_ref.is_empty() {
        return Err(ContractError::ReferenceEmpty);
    }
    if settlement_ref.len() > MAX_SETTLEMENT_REF_LEN {
        return Err(ContractError::ReferenceTooLong);
    }
    Ok(settlement_ref.to_string())
}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coin_marker_rejects_forced_transfer() {
        let err = validate_ldt_marker_policy(LdtMarkerType::Coin, true, &[]).unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidLdtMarkerPolicy(ref reason)
                if reason.contains("allow_forced_transfer")
        ));
    }

    #[test]
    fn coin_marker_rejects_required_attributes() {
        let err = validate_ldt_marker_policy(
            LdtMarkerType::Coin,
            false,
            &["loan-dicer.pi-holder".to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidLdtMarkerPolicy(ref reason)
                if reason.contains("required_attributes")
        ));
    }

    #[test]
    fn restricted_marker_normalizes_and_accepts_valid_attributes() {
        let attrs = validate_ldt_marker_policy(
            LdtMarkerType::Restricted,
            true,
            &[
                " Loan-Dicer.Pi-Holder ".to_string(),
                "*.approved.example".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            attrs,
            vec![
                "loan-dicer.pi-holder".to_string(),
                "*.approved.example".to_string()
            ]
        );
    }

    #[test]
    fn rejects_duplicate_required_attributes() {
        let err = validate_ldt_marker_policy(
            LdtMarkerType::Restricted,
            false,
            &[
                "loan-dicer.pi-holder".to_string(),
                "Loan-Dicer.pi-holder".to_string(),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ContractError::InvalidLdtMarkerPolicy(ref reason)
                if reason.contains("duplicate") && reason.contains("loan-dicer.pi-holder")
        ));
    }

    #[test]
    fn rejects_empty_or_malformed_required_attributes() {
        for attribute in ["", "   ", "foo..bar", "*kyc.example", "kyc.*.example", "*"] {
            let err = validate_ldt_marker_policy(
                LdtMarkerType::Restricted,
                false,
                &[attribute.to_string()],
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    ContractError::InvalidLdtMarkerPolicy(ref reason) if reason.contains(attribute)
                ),
                "expected invalid attribute for {attribute:?}"
            );
        }
    }
}
