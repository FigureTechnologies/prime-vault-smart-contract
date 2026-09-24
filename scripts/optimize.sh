#!/bin/sh
# Build a CosmWasm-optimized artifact with wasm-opt inlining capped.
#
# cosmwasm/optimizer:0.17.0 runs `wasm-opt -Os`, and Binaryen 116 defaults
# `--one-caller-inline-max-function-size` to -1 (inline every single-caller
# function). That merges CleanupOrphanMarkers into one function with >100
# locals, which Provenance's CosmWasm VM rejects. Cap inlining at 15, the
# historical -Os value, so the split handlers stay under the limit.
set -eu

docker run --rm \
  -v "$(pwd)":/code \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  --entrypoint ash \
  cosmwasm/optimizer:0.17.0 \
  -c '
    set -eu
    cd /code
    /usr/local/bin/bob
    mkdir -p artifacts
    for WASM in /target/wasm32-unknown-unknown/release/*.wasm; do
      [ -e "$WASM" ] || continue
      OUT="artifacts/$(basename "$WASM")"
      echo "Optimizing $(basename "$WASM") ..."
      wasm-opt -Os --one-caller-inline-max-function-size=15 "$WASM" -o "$OUT"
    done
    cd artifacts
    sha256sum -- *.wasm | tee checksums.txt
  '
