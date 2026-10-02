#!/bin/sh
# Build the wasm QR decoder and install it into the sendmecongo-mp miniprogram.
#
#   tools/build-decoder-wasm.sh
#
# 1. compiles crates/sendmecongo-decoder-wasm for wasm32-unknown-unknown
#    (profile `wasm-release`, defined at the workspace root)
# 2. copies the .wasm into sendmecongo-mp/libs/decoder.wasm
#
# The miniprogram loads it with WXWebAssembly.instantiate('libs/decoder.wasm'),
# which ONLY accepts a code-package file path (no ArrayBuffer, no base64) and
# requires base library >= 2.13.0. Standard `WebAssembly` does not exist in the
# miniprogram logic layer at all.
set -eu
cd "$(dirname "$0")/.."

TARGET_DIR=target/wasm32-unknown-unknown/wasm-release
OUT_WASM=../sendmecongo-mp/libs/decoder.wasm

cargo build -p sendmecongo-decoder-wasm --target wasm32-unknown-unknown --profile wasm-release

mkdir -p "$(dirname "$OUT_WASM")"
cat "$TARGET_DIR/sendmecongo_decoder_wasm.wasm" > "$OUT_WASM"

echo "wrote $OUT_WASM ($(wc -c < "$OUT_WASM" | tr -d ' ') bytes)"
