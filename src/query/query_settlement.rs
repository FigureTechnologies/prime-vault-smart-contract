use cosmwasm_std::Deps;

use crate::error::ContractError;
use crate::msg::GetSettlementResponse;
use crate::state::{normalize_settlement_ref, SETTLEMENTS};

pub fn query_settlement(
    deps: Deps,
    settlement_ref: String,
) -> Result<GetSettlementResponse, ContractError> {
    let settlement_ref = normalize_settlement_ref(&settlement_ref)?;
    let settlement = SETTLEMENTS
        .may_load(deps.storage, settlement_ref.clone())?
        .ok_or_else(|| ContractError::SettlementNotFound {
            settlement_ref: settlement_ref.clone(),
        })?;
    Ok(GetSettlementResponse {
        settlement_ref,
        total_pi_burned: settlement.total_pi_burned,
        holder_count: settlement.holder_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCtx;

    #[test]
    fn missing_settlement_is_not_found() {
        let ctx = TestCtx::new();
        let err = query_settlement(ctx.deps.as_ref(), "missing".to_string()).unwrap_err();
        assert!(matches!(
            err,
            ContractError::SettlementNotFound { settlement_ref } if settlement_ref == "missing"
        ));
    }
}
