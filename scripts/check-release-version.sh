#!/bin/sh
# Checks that a release tag matches the package version, so a v0.2.0 tag can't
# publish a binary that still calls itself 0.1.0.
#
#   scripts/check-release-version.sh <tag>
#
# The tag must be the version in Cargo.toml with a leading v, Cargo.lock must
# agree with Cargo.toml, and CHANGELOG.md must have notes for the version. Used by the version job in .github/workflows/ci.yml and
# by `just release-check`.
set -eu

tag=$1

# The first version line in the [package] table.
manifest=$(awk '
	/^\[/ { in_package = ($0 == "[package]") }
	in_package && /^version *=/ { gsub(/^version *= *"|".*$/, ""); print; exit }
' Cargo.toml)

# The version of the gol-bevy entry, from the line after its name.
lock=$(awk '
	$0 == "name = \"gol-bevy\"" { found = 1; next }
	found && /^version *=/ { gsub(/^version *= *"|".*$/, ""); print; exit }
' Cargo.lock)

if [ -z "$manifest" ]; then
	echo "error: no version found in Cargo.toml's [package] table" >&2
	exit 1
fi
if [ "$lock" != "$manifest" ]; then
	echo "error: Cargo.lock has gol-bevy ${lock:-missing}, but Cargo.toml has $manifest; run cargo build to update it" >&2
	exit 1
fi
if [ "$tag" != "v$manifest" ]; then
	echo "error: tag $tag does not match Cargo.toml's version $manifest; expected v$manifest" >&2
	exit 1
fi
scripts/release-notes.sh "$manifest" >/dev/null
echo "Tag $tag matches version $manifest, and CHANGELOG.md has notes for it"
