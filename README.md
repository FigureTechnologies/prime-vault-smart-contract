# Loan Dicer Smart Contract

A CosmWasm smart contract (with Provenance extensions) that manages **loan dicer token (LDT)** contributions, redemptions, and swaps. The contract keeps track of pending records, validates scope ownership during pooling, and mints/burns tokens via Provenance markers.

## ✨ Highlights

- **Contributions**: OTC initiates a contribution for a designated contributor. The contributor submits a restricted marker; the OTC prices it in LDT; the contributor confirms that price to settle.
- **Redemptions**: OTC initiates a pool and moves contract-held scopes onto it, then marks pooling complete with a burn amount. The designated redeemer confirms by sending LDT.
- **Swaps**: OTC initiates a pool and moves removed scopes onto it, then marks pooling complete with mint/burn amounts. The designated contributor submits incoming scopes; OTC confirms.
- **Marker integration**: Uses Provenance markers for LDT and redemption/swap pooling.
- **Reference lookup**: Optional human-friendly references for contributions, redemptions, and swaps.

## Contract State (high-level)

- `CONFIGURATION`: OTC, LDT denom, redemption marker admin, optional `app_metadata` (set at instantiate)
- `PENDING_CONTRIBUTIONS`: pending contributions by ID (`contributor_addr`, optional `marker_denom` after submit, optional `mint_ldt_amount` after pricing)
- `PENDING_REDEMPTIONS`: pending redemptions by ID (`pool_denom`, `pooling_complete`, optional `burn_ldt_amount` after pool complete, `redeemer_addr`)
- `PENDING_SWAPS`: pending swaps by ID (`pool_denom`, `pooling_complete`, mint/burn amounts after pool complete, `contributor_addr`, optional `incoming_marker_denom` after submit)
- `CONTRIBUTION_REFERENCES`: optional reference → contribution ID
- `REDEMPTION_REFERENCES`: optional reference → redemption ID
- `SWAP_REFERENCES`: optional reference → swap ID

References are optional, but when supplied must be non-empty (whitespace-only is rejected), at most 128 characters, and must not collide with a live record's reference. A reference is released when its record completes or cancels, so treat it as unique only while the record is live.

## Instantiate

Message: `InstantiateContractMsg`

```json
{
  "otc_address": "...",
  "ldt_denom": "...",
  "redemption_marker_admin": "...",
  "app_metadata": null
}
```

(`app_metadata` is optional; when set, `kind` is limited to 64 characters and `data` to 1024.)

On instantiation, the contract stores configuration and creates the LDT marker with the contract as admin.

## Execute Messages

- `InitiateContribution { contributor_address, contribution_ref }`
- `SubmitContribution { contribution_id, marker_denom }`
- `PriceContribution { contribution_id, mint_ldt_amount }` — OTC offers a price for the escrowed marker. Nothing mints here, and re-pricing is allowed because the offer is not binding until confirmed.
- `ConfirmContribution { contribution_id, expected_mint_ldt_amount }` — contributor only. Settles the contribution at the price they read from `GetPendingContribution`; fails if the OTC re-priced in the meantime.
- `CancelContribution { contribution_id }`
- `InitiateRedemption { pool_denom, num_coins, redeemer_address, redemption_ref }`
- `PoolRedemption { redemption_id, scope_uuids }` — OTC moves the given contract-held scopes onto the redemption pool marker
- `CompleteRedemptionPool { redemption_id, burn_ldt_amount }` — write-once: rejected once pooling is already complete, so the burn amount cannot change after the redeemer has seen it. Cancel and re-initiate to correct a mistake.
- `ConfirmRedemption { redemption_id }` — redeemer sends `burn_ldt_amount` LDT with this message
- `CancelRedemption { redemption_id }`
- `CleanupOrphanMarkers { marker_denoms }` — contract admin only
- `UpdateConfiguration { otc_address, redemption_marker_admin }` — contract admin only. Rotates either or both privileged addresses; omitted fields are left unchanged and at least one must be supplied. `ldt_denom` is intentionally not rotatable. See [Rotating privileged addresses](#rotating-privileged-addresses).
- `InitiateSwap { pool_denom, num_coins, contributor_address, swap_ref }`
- `PoolSwap { swap_id, removed_scope_uuids }` — OTC moves the given contract-held scopes onto the swap pool marker
- `CompleteSwapPool { swap_id, mint_ldt_amount, burn_ldt_amount }` — mint and burn are mutually exclusive (both zero is valid). Write-once: rejected once pooling is already complete, so the contributor's terms cannot be changed after they are published. Cancel and re-initiate to correct a mistake.
- `SubmitSwap { swap_id, incoming_marker_denom }` — contributor must attach `burn_ldt_amount` LDT when burn is set
- `ConfirmSwap { swap_id }`
- `CancelSwap { swap_id }`

See `src/msg.rs` for full schema and docs.

### Restricted-marker authz

`SubmitContribution` and `SubmitSwap` escrow a restricted marker by transferring the contributor's coins into the marker account (`MsgTransferRequest` with administrator = contract, from = contributor). Before calling either route, the contributor must grant this contract `MarkerTransferAuthorization` (authz) covering the full marker holding, and the contract must already have Admin on the marker. Without the grant, the marker keeper rejects the transfer.

### Rotating privileged addresses

`UpdateConfiguration` lets the contract admin replace `otc_address` and `redemption_marker_admin` without a code migration. This matters because every initiate, pool, complete, and confirm route is gated on `otc_address`: if that key were lost with no rotation route, all contract-held assets would be stranded, and migrating out is blocked while any pending record exists.

Two things behave differently between the fields:

- **`otc_address` takes effect immediately**, including for records already in flight, because every privileged route reads it at call time. The retired key stops working on the next block.
- **`redemption_marker_admin` applies only to redemptions and swaps confirmed after the call.** Pool markers that already settled granted custody to the previous admin, and rotating here does not revisit them. Those are recoverable through governance, which is why pool markers are created with `allow_governance_control: true`.

`ldt_denom` is deliberately not rotatable — it is bound to the marker created at instantiate, so changing it would orphan that marker along with the contract's mint and burn authority.

### The contract address is a coin sink

**Never send coins directly to the contract address.** Anything that arrives by a plain bank transfer is permanently unrecoverable.

The contract has no sweep route, and this is deliberate. The only coins it is designed to hold are the burn LDT a contributor attaches to `SubmitSwap`, which `ConfirmSwap` burns in the same flow — every inflow has a matching outflow, so the balance is transient by design. Every execute route that does not take payment rejects attached funds outright, so the only way to strand coins here is a bank `MsgSend`, which never executes contract code and therefore cannot be refused.

Recovering coins stranded this way would require deploying and migrating to a new code version that adds a withdrawal path. Treat the balance as unrecoverable when reconciling operationally.

## Migrate

After storing upgraded WASM, migrate an existing instance (contract admin only):

```json
{}
```

Message type: `MigrateMsg`.

From version **0.3.0**, migration requires that there are no pending contributions, redemptions, or swaps (complete or cancel them first) and sets the cw2 contract version to **1.0.0**. Contracts below **0.3.0** cannot migrate with this code. Re-running migrate when already at **1.0.0** is allowed when pending maps are still empty (version is refreshed).

Once the pending maps are confirmed empty, migrate also clears the `PENDING_DENOMS` reverse index, since any entry left there is stale by definition. The number removed is reported in the `cleared_pending_denoms` attribute — a non-zero value means a denom had leaked and would otherwise have stayed fenced off permanently, so it is worth investigating even though the migration repairs it.

## Query Messages

- `GetPendingContribution { contribution_id }` — `{ contribution_id, contributor_addr, marker_denom, mint_ldt_amount }`
- `GetPendingContributionByReference { contribution_ref }` — same shape as above
- `GetPendingRedemption { redemption_id }` — `{ redemption_id, burn_ldt_amount, pool_denom, pooling_complete, redeemer_addr }`
- `GetPendingRedemptionByReference { redemption_ref }`
- `GetPendingSwap { swap_id }` — `{ swap_id, pool_denom, pooling_complete, contributor_addr, mint_ldt_amount, burn_ldt_amount, incoming_marker_denom }`
- `GetPendingSwapByReference { swap_ref }`
- `GetConfiguration {}` — includes `app_metadata` when set at instantiate

## Build

```bash
cargo build
```

### Build WASM (local)

```bash
cargo build --release --target wasm32-unknown-unknown
```

### Build WASM (optimizer)

Use the repo script rather than the stock `cosmwasm/optimizer` entrypoint. That image runs `wasm-opt -Os`, which inlines every single-caller function and can produce a Wasm function with more than 100 locals — Provenance rejects that at store time.

```bash
./scripts/optimize.sh
```

or, equivalently:

```bash
cargo run-script optimize
```

> Both invoke `scripts/optimize.sh`, which uses `cosmwasm/optimizer:0.17.0` with `--one-caller-inline-max-function-size=15`.

## Tests

```bash
cargo test
```

## License

See `LICENSE`.
