use cosmwasm_std::MessageInfo;

use crate::error::ContractError;

/// Rejects coins attached to a route that does not accept payment.
/// CosmWasm deposits `info.funds` into the contract on success, and there is no
/// general withdrawal path — unexpected funds would be permanently lost.
///
/// The absence of a sweep route is deliberate: the contract address is documented as a coin
/// sink. This guard is therefore the only defence, and it cannot cover a direct bank `MsgSend`,
/// which never executes contract code. Coins that arrive that way are unrecoverable short of a
/// code migration. Keep this check on every route that does not take payment.
pub fn assert_no_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if !info.funds.is_empty() {
        Err(ContractError::InvalidFundsSent)
    } else {
        Ok(())
    }
}

pub mod admin {
    pub mod cleanup_orphan_markers;
    pub mod update_configuration;
}
pub mod contribution {
    pub mod cancel_contribution;
    pub mod confirm_contribution;
    pub mod initiate_contribution;
    pub mod price_contribution;
    pub mod submit_contribution;
}
pub mod redemption {
    pub mod cancel_redemption;
    pub mod complete_redemption_pool;
    pub mod confirm_redemption;
    pub mod initiate_redemption;
    pub mod pool_redemption;
}
pub mod swap {
    pub mod cancel_swap;
    pub mod complete_swap_pool;
    pub mod confirm_swap;
    pub mod initiate_swap;
    pub mod pool_swap;
    pub mod submit_swap;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{testing::message_info, Addr, Coin, Uint128};

    #[test]
    fn accepts_empty_funds() {
        let info = message_info(&Addr::unchecked("sender"), &[]);
        assert!(assert_no_funds(&info).is_ok());
    }

    #[test]
    fn rejects_attached_funds() {
        let info = message_info(
            &Addr::unchecked("sender"),
            &[Coin {
                denom: "nhash".to_string(),
                amount: Uint128::new(1),
            }],
        );
        assert!(matches!(
            assert_no_funds(&info).unwrap_err(),
            ContractError::InvalidFundsSent
        ));
    }
}
