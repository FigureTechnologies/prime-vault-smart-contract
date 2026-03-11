# Loan Dicer Smart Contract

A CosmWasm smart contract (with Provenance extensions) that manages **loan dicer token (LDT)** contributions and redemptions. The contract keeps track of pending contributions/redemptions, validates scope ownership, and mints/burns tokens via Provenance markers.

## ✨ Highlights

- **Contributions**: OTC initiates and finalizes contributions to mint LDT.
- **Redemptions**: Token holders initiate redemptions to receive underlying scopes, with pooling support.
- **Marker integration**: Uses Provenance markers for LDT and redemption pooling.
- **Reference lookup**: Optional human-friendly references for contributions and redemptions.

## Contract State (high-level)

- `CONFIGURATION`: OTC, trust address, LDT denom, redemption marker admin
- `PENDING_CONTRIBUTIONS`: pending contributions by ID
- `PENDING_REDEMPTIONS`: pending redemptions by ID
- `CONTRIBUTION_REFERENCES`: optional reference → contribution ID
- `REDEMPTION_REFERENCES`: optional reference → redemption ID

## Instantiate

Message: `InstantiateContractMsg`

```json
{
  "otc_address": "...",
  "trust_address": "...",
  "ldt_denom": "...",
  "redemption_marker_admin": "..."
}
```

On instantiation, the contract stores configuration and creates the LDT marker with the contract as admin.

## Execute Messages

- `InitiateContribution { scope_uuids, mint_ldt_amount, contribution_ref }`
- `FinalizeContribution { contribution_id, marker_denom }`
- `CancelContribution { contribution_id }`
- `InitiateRedemption { burn_ldt_amount, scope_uuids, pool_denom, num_coins, redeemer_address, redemption_ref }`
- `PoolRedemption { redemption_id, num_scopes_to_pool }`
- `FinalizeRedemption { redemption_id }`
- `CancelRedemption { redemption_id }`

See `src/msg.rs` for full schema and docs.

## Query Messages

- `GetPendingContribution { contribution_id }`
- `GetPendingContributionByReference { contribution_ref }`
- `GetPendingRedemption { redemption_id }`
- `GetPendingRedemptionByReference { redemption_ref }`
- `GetConfiguration {}`

## Build

```bash
cargo build
```

### Build WASM (local)

```bash
cargo build --release --target wasm32-unknown-unknown
```

### Build WASM (optimizer)

```bash
docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.17.0
```

> This command matches the optimizer script defined in `Cargo.toml` under `[package.metadata.scripts]`.

## Tests

```bash
cargo test
```

## License

See `LICENSE`.
