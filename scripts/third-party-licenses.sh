#!/bin/sh
# Writes the licence notices of every crate compiled into gol-bevy, on every
# platform it is built for, as an HTML page. Needs cargo-about:
#
#   cargo install cargo-about --locked --features cli
#
#   scripts/third-party-licenses.sh [output]   (default: dist/THIRD-PARTY-LICENSES.html)
#
# Fails if a dependency uses a licence not accepted in about.toml.
set -eu

out=${1:-dist/THIRD-PARTY-LICENSES.html}
mkdir -p "$(dirname "$out")"
cargo about generate --locked --fail about.hbs -o "$out"
echo "$out"
