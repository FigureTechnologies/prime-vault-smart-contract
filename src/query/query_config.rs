use cosmwasm_std::Deps;

use crate::error::ContractError;
use crate::state::CONFIGURATION;

pub fn query_configuration(deps: Deps) -> Result<crate::state::Configuration, ContractError> {
    Ok(CONFIGURATION.load(deps.storage)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestCtx, LDT_DENOM};

    #[test]
    fn returns_the_stored_configuration() {
        let ctx = TestCtx::new();
        let cfg = query_configuration(ctx.deps.as_ref()).unwrap();
        assert_eq!(cfg.otc_address, ctx.otc);
        assert_eq!(cfg.redemption_marker_admin, ctx.redemption_admin);
        assert_eq!(cfg.ldt_denom, LDT_DENOM);
        assert!(cfg.app_metadata.is_none());
    }
}
