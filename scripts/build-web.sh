#!/usr/bin/env bash
# Builds the browser version into a folder of static files: web/index.html, the
# JavaScript glue, and the WebAssembly module. Serve the folder over HTTP; a
# file:// URL cannot load the module.
#
# Usage: scripts/build-web.sh [out-dir]   (default: dist/web)
#
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli at the same
# version as the wasm-bindgen crate in Cargo.lock. wasm-opt, from binaryen, is
# used to shrink the module when it is installed.
set -euo pipefail

out_dir=${1:-dist/web}
target=wasm32-unknown-unknown

# The CLI and the crate must agree exactly, or the glue does not match the module.
locked=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
installed=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [[ "$installed" != "$locked" ]]; then
  echo "error: need wasm-bindgen-cli $locked, found ${installed:-none}." >&2
  echo "Install it with: cargo install wasm-bindgen-cli --version $locked --locked" >&2
  exit 1
fi

rustup target add "$target"
cargo build --profile web-release --locked --target "$target"

rm -rf "$out_dir"
mkdir -p "$out_dir"
wasm-bindgen --target web --no-typescript --out-dir "$out_dir" --out-name gol-bevy \
  "target/$target/web-release/gol-bevy.wasm"

module="$out_dir/gol-bevy_bg.wasm"
if command -v wasm-opt >/dev/null; then
  # The module's target_features section tells wasm-opt which features it may use.
  wasm-opt -Os -o "$module" "$module"
else
  echo "note: wasm-opt not found; the module is left unoptimized." >&2
fi

cp web/index.html "$out_dir/"
echo "Built $out_dir ($(du -h "$module" | cut -f1) module)."
