use std::collections::HashMap;

use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{
    Addr, Binary, Coin as StdCoin, ContractResult, CosmosMsg, Env, MessageInfo, OwnedDeps,
    Response, SystemResult, Uint128,
};
use provwasm_common::MockableQuerier;
use provwasm_mocks::{mock_provenance_dependencies, MockProvenanceQuerier};
use provwasm_std::shim::Any;
use provwasm_std::types::cosmos::auth::v1beta1::BaseAccount;
use provwasm_std::types::cosmos::base::v1beta1::Coin;
use provwasm_std::types::provenance::marker::v1::{
    AccessGrant, Balance, MarkerAccount, MarkerStatus, MarkerType, QueryHoldingRequest,
    QueryHoldingResponse, QueryMarkerRequest, QueryMarkerResponse,
};
use provwasm_std::types::tendermint::abci::ResponseQuery;

use crate::state::{Configuration, CONFIGURATION};

pub const LDT_DENOM: &str = "ldt.token.test";
pub const MARKER_DENOM: &str = "incoming.marker";
pub const POOL_DENOM: &str = "pool.marker";

pub struct TestCtx {
    pub deps: OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        MockProvenanceQuerier,
    >,
    pub env: Env,
    pub otc: Addr,
    pub contributor: Addr,
    pub redeemer: Addr,
    pub redemption_admin: Addr,
    pub stranger: Addr,
    markers: HashMap<String, MarkerMock>,
    scopes: HashMap<String, ScopeMock>,
}

#[derive(Clone)]
struct ScopeMock {
    value_owner: String,
    scope_addr: String,
}

#[derive(Clone)]
pub struct MarkerMock {
    pub denom: String,
    pub marker_address: String,
    pub holders: Vec<(String, String)>,
    pub access: Vec<AccessGrant>,
    pub status: i32,
    pub marker_type: i32,
}

impl TestCtx {
    pub fn new() -> Self {
        let deps = mock_provenance_dependencies();
        let env = mock_env();
        let otc = deps.api.addr_make("otc");
        let contributor = deps.api.addr_make("contributor");
        let redeemer = deps.api.addr_make("redeemer");
        let redemption_admin = deps.api.addr_make("redemption_marker_admin");
        let stranger = deps.api.addr_make("stranger");

        let mut ctx = Self {
            deps,
            env,
            otc,
            contributor,
            redeemer,
            redemption_admin,
            stranger,
            markers: HashMap::new(),
            scopes: HashMap::new(),
        };
        ctx.save_config();
        ctx
    }

    pub fn save_config(&mut self) {
        CONFIGURATION
            .save(
                &mut self.deps.storage,
                &Configuration {
                    otc_address: self.otc.clone(),
                    ldt_denom: LDT_DENOM.to_string(),
                    redemption_marker_admin: self.redemption_admin.clone(),
                    app_metadata: None,
                },
            )
            .unwrap();
    }

    pub fn otc_info(&self) -> MessageInfo {
        message_info(&self.otc, &[])
    }

    pub fn otc_info_with_funds(&self, funds: &[StdCoin]) -> MessageInfo {
        message_info(&self.otc, funds)
    }

    pub fn contributor_info(&self) -> MessageInfo {
        message_info(&self.contributor, &[])
    }

    pub fn contributor_info_with_funds(&self, funds: &[StdCoin]) -> MessageInfo {
        message_info(&self.contributor, funds)
    }

    /// Dummy LDT attachment used to assert that non-payment routes reject funds.
    pub fn unexpected_funds() -> [StdCoin; 1] {
        [StdCoin {
            denom: LDT_DENOM.to_string(),
            amount: Uint128::new(1),
        }]
    }

    pub fn redeemer_info_with_funds(&self, funds: &[StdCoin]) -> MessageInfo {
        message_info(&self.redeemer, funds)
    }

    pub fn stranger_info(&self) -> MessageInfo {
        message_info(&self.stranger, &[])
    }

    pub fn contract_addr(&self) -> String {
        self.env.contract.address.to_string()
    }

    pub fn mock_contract_admin(&mut self, admin: Addr) {
        use cosmwasm_std::{to_json_binary, ContractInfoResponse, SystemError, WasmQuery};

        let contract_addr = self.env.contract.address.clone();
        self.deps
            .querier
            .mock_querier
            .update_wasm(move |query| match query {
                WasmQuery::ContractInfo { .. } => SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&ContractInfoResponse::new(
                        1,
                        contract_addr.clone(),
                        Some(admin.clone()),
                        false,
                        None,
                    ))
                    .unwrap(),
                )),
                _ => SystemResult::Err(SystemError::UnsupportedRequest {
                    kind: "wasm".to_string(),
                }),
            });
    }

    /// Contract info with no admin set. Admin routes must reject rather than treat that as open access.
    pub fn mock_contract_without_admin(&mut self) {
        use cosmwasm_std::{to_json_binary, ContractInfoResponse, SystemError, WasmQuery};

        let contract_addr = self.env.contract.address.clone();
        self.deps
            .querier
            .mock_querier
            .update_wasm(move |query| match query {
                WasmQuery::ContractInfo { .. } => SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&ContractInfoResponse::new(
                        1,
                        contract_addr.clone(),
                        None,
                        false,
                        None,
                    ))
                    .unwrap(),
                )),
                _ => SystemResult::Err(SystemError::UnsupportedRequest {
                    kind: "wasm".to_string(),
                }),
            });
    }

    pub fn mock_empty_value_ownership(&mut self) {
        use provwasm_std::types::provenance::metadata::v1::ValueOwnershipResponse;

        self.deps.querier.register_custom_query(
            "/provenance.metadata.v1.Query/ValueOwnership".to_string(),
            Box::new(move |_data| {
                grpc_ok(
                    ValueOwnershipResponse {
                        scope_uuids: vec![],
                        request: None,
                        pagination: None,
                    }
                    .to_proto_bytes(),
                )
            }),
        );
    }

    /// Value ownership that reports scopes on the first page, with no further pages.
    pub fn mock_value_ownership_with_scopes(&mut self, scope_uuids: Vec<String>) {
        use provwasm_std::types::provenance::metadata::v1::ValueOwnershipResponse;

        self.deps.querier.register_custom_query(
            "/provenance.metadata.v1.Query/ValueOwnership".to_string(),
            Box::new(move |_data| {
                grpc_ok(
                    ValueOwnershipResponse {
                        scope_uuids: scope_uuids.clone(),
                        request: None,
                        pagination: None,
                    }
                    .to_proto_bytes(),
                )
            }),
        );
    }

    /// Value ownership that always reports an empty page while handing back the same `next_key`,
    /// simulating a node that never advances the cursor.
    pub fn mock_value_ownership_stuck_pagination(&mut self) {
        use provwasm_std::types::cosmos::base::query::v1beta1::PageResponse;
        use provwasm_std::types::provenance::metadata::v1::ValueOwnershipResponse;

        self.deps.querier.register_custom_query(
            "/provenance.metadata.v1.Query/ValueOwnership".to_string(),
            Box::new(move |_data| {
                grpc_ok(
                    ValueOwnershipResponse {
                        scope_uuids: vec![],
                        request: None,
                        pagination: Some(PageResponse {
                            next_key: Some(vec![7, 7, 7]),
                            total: 0,
                        }),
                    }
                    .to_proto_bytes(),
                )
            }),
        );
    }

    pub fn set_contract_bank_balance(&mut self, denom: &str, amount: u128) {
        self.set_contract_bank_balances(&[(denom, amount)]);
    }

    /// Replaces the contract's mocked bank balance. Pass every denom in one call;
    /// a later call overwrites the previous set.
    pub fn set_contract_bank_balances(&mut self, balances: &[(&str, u128)]) {
        self.deps.querier.mock_querier.bank.update_balance(
            self.contract_addr(),
            balances
                .iter()
                .map(|(denom, amount)| cosmwasm_std::Coin::new(*amount, *denom))
                .collect(),
        );
    }

    pub fn mock_restricted_marker(
        &mut self,
        denom: &str,
        holder: &str,
        amount: &str,
        access: Vec<AccessGrant>,
    ) {
        self.mock_marker(
            denom,
            holder,
            amount,
            access,
            MarkerStatus::Active,
            MarkerType::Restricted,
        );
    }

    /// Restricted marker whose holding lists every `(address, amount)` pair.
    /// Use this when a test needs zero holders or more than one.
    pub fn mock_restricted_marker_holders(&mut self, denom: &str, holders: Vec<(&str, &str)>) {
        self.markers.insert(
            denom.to_string(),
            MarkerMock {
                denom: denom.to_string(),
                marker_address: format!("{denom}.marker.account"),
                holders: holders
                    .into_iter()
                    .map(|(address, amount)| (address.to_string(), amount.to_string()))
                    .collect(),
                access: vec![],
                status: MarkerStatus::Active as i32,
                marker_type: MarkerType::Restricted as i32,
            },
        );
        self.reregister_marker_queries();
    }

    pub fn mock_restricted_marker_with_address(
        &mut self,
        denom: &str,
        marker_address: &str,
        holder: &str,
        amount: &str,
        access: Vec<AccessGrant>,
    ) {
        self.mock_marker_with_address(
            denom,
            marker_address,
            holder,
            amount,
            access,
            MarkerStatus::Active,
            MarkerType::Restricted,
        );
    }

    pub fn mock_marker(
        &mut self,
        denom: &str,
        holder: &str,
        amount: &str,
        access: Vec<AccessGrant>,
        status: MarkerStatus,
        marker_type: MarkerType,
    ) {
        self.mock_marker_with_address(
            denom,
            &format!("{denom}.marker.account"),
            holder,
            amount,
            access,
            status,
            marker_type,
        );
    }

    pub fn mock_marker_with_address(
        &mut self,
        denom: &str,
        marker_address: &str,
        holder: &str,
        amount: &str,
        access: Vec<AccessGrant>,
        status: MarkerStatus,
        marker_type: MarkerType,
    ) {
        self.markers.insert(
            denom.to_string(),
            MarkerMock {
                denom: denom.to_string(),
                marker_address: marker_address.to_string(),
                holders: vec![(holder.to_string(), amount.to_string())],
                access,
                status: status as i32,
                marker_type: marker_type as i32,
            },
        );
        self.reregister_marker_queries();
    }

    /// Mock a metadata scope so `get_nft` can resolve `value_owner` and the nft denom.
    pub fn mock_scope(&mut self, scope_uuid: &str, value_owner: &str, scope_addr: &str) {
        self.scopes.insert(
            scope_uuid.to_lowercase(),
            ScopeMock {
                value_owner: value_owner.to_string(),
                scope_addr: scope_addr.to_string(),
            },
        );
        self.reregister_scope_queries();
    }

    fn reregister_scope_queries(&mut self) {
        use provwasm_std::types::provenance::metadata::v1::{
            Scope, ScopeIdInfo, ScopeRequest, ScopeResponse, ScopeWrapper,
        };

        let scopes = self.scopes.clone();
        self.deps.querier.register_custom_query(
            "/provenance.metadata.v1.Query/Scope".to_string(),
            Box::new(move |data| {
                let req = ScopeRequest::try_from(data.clone()).expect("scope request");
                let scope = scopes
                    .get(&req.scope_id.to_lowercase())
                    .unwrap_or_else(|| panic!("no mocked scope for {}", req.scope_id));
                grpc_ok(
                    ScopeResponse {
                        scope: Some(ScopeWrapper {
                            scope: Some(Scope {
                                scope_id: vec![],
                                specification_id: vec![],
                                owners: vec![],
                                data_access: vec![],
                                value_owner_address: scope.value_owner.clone(),
                                require_party_rollup: false,
                            }),
                            scope_id_info: Some(ScopeIdInfo {
                                scope_id: vec![],
                                scope_id_prefix: vec![],
                                scope_id_scope_uuid: vec![],
                                scope_addr: scope.scope_addr.clone(),
                                scope_uuid: req.scope_id.clone(),
                            }),
                            scope_spec_id_info: None,
                        }),
                        sessions: vec![],
                        records: vec![],
                        request: None,
                    }
                    .to_proto_bytes(),
                )
            }),
        );
    }

    fn reregister_marker_queries(&mut self) {
        let markers = self.markers.clone();

        let marker_lookup = markers.clone();
        self.deps.querier.register_custom_query(
            "/provenance.marker.v1.Query/Marker".to_string(),
            Box::new(move |data| {
                let req = QueryMarkerRequest::try_from(data.clone()).expect("marker request");
                let marker = lookup_marker(&marker_lookup, &req.id);
                grpc_ok(
                    QueryMarkerResponse {
                        marker: Some(Any {
                            type_url: MarkerAccount::TYPE_URL.to_string(),
                            value: marker_account(marker).to_proto_bytes(),
                        }),
                    }
                    .to_proto_bytes(),
                )
            }),
        );

        let holding_lookup = markers;
        self.deps.querier.register_custom_query(
            "/provenance.marker.v1.Query/Holding".to_string(),
            Box::new(move |data| {
                let req = QueryHoldingRequest::try_from(data.clone()).expect("holding request");
                let marker = lookup_marker(&holding_lookup, &req.id);
                grpc_ok(
                    QueryHoldingResponse {
                        balances: marker
                            .holders
                            .iter()
                            .map(|(address, amount)| Balance {
                                address: address.clone(),
                                coins: vec![Coin {
                                    denom: marker.denom.clone(),
                                    amount: amount.clone(),
                                }],
                            })
                            .collect(),
                        pagination: None,
                    }
                    .to_proto_bytes(),
                )
            }),
        );
    }
}

/// Provenance marker queries parse `id` as a bech32 address first, then as a denom.
fn lookup_marker<'a>(markers: &'a HashMap<String, MarkerMock>, id: &str) -> &'a MarkerMock {
    markers
        .values()
        .find(|marker| marker.marker_address == id)
        .or_else(|| markers.get(id))
        .unwrap_or_else(|| panic!("no mocked marker for {id}"))
}

fn marker_account(marker: &MarkerMock) -> MarkerAccount {
    MarkerAccount {
        base_account: Some(BaseAccount {
            address: marker.marker_address.clone(),
            pub_key: None,
            account_number: 1,
            sequence: 0,
        }),
        manager: String::new(),
        access_control: marker.access.clone(),
        status: marker.status,
        denom: marker.denom.clone(),
        supply: "1".to_string(),
        marker_type: marker.marker_type,
        supply_fixed: false,
        allow_governance_control: false,
        allow_forced_transfer: false,
        required_attributes: vec![],
    }
}

fn grpc_ok(value: Vec<u8>) -> cosmwasm_std::QuerierResult {
    SystemResult::Ok(ContractResult::Ok(Binary::new(
        ResponseQuery {
            code: 0,
            log: String::new(),
            info: String::new(),
            index: 0,
            key: vec![],
            value,
            proof_ops: None,
            height: 0,
            codespace: String::new(),
        }
        .to_proto_bytes(),
    )))
}

pub fn contract_access(contract: &str, permissions: Vec<i32>) -> AccessGrant {
    AccessGrant {
        address: contract.to_string(),
        permissions,
    }
}

pub fn typed_msgs<T>(res: &Response, type_url: &str) -> Vec<T>
where
    T: TryFrom<Binary>,
    <T as TryFrom<Binary>>::Error: std::fmt::Debug,
{
    res.messages
        .iter()
        .filter_map(|msg| match &msg.msg {
            CosmosMsg::Any(any) if any.type_url == type_url => {
                Some(T::try_from(any.value.clone()).unwrap())
            }
            _ => None,
        })
        .collect()
}

pub fn attr<'a>(res: &'a Response, key: &str) -> &'a str {
    res.attributes
        .iter()
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("missing attribute {key}"))
        .value
        .as_str()
}
