#!/bin/sh
# Packages a release binary with the README and licence into dist/.
#
#   scripts/package-release.sh <archive name> <binary> [tar.gz|zip]
#
# Used by the release jobs in .github/workflows/ci.yml and by `just release-macos`, so
# every platform's download has the same layout.
set -eu

name=$1
binary=$2
format=${3:-tar.gz}

staging=dist/$name
rm -rf "$staging"
mkdir -p "$staging"
cp "$binary" README.md LICENSE "$staging/"

case $format in
tar.gz) tar -C dist -czf "dist/$name.tar.gz" "$name" ;;
zip) (cd dist && rm -f "$name.zip" && zip -qr "$name.zip" "$name") ;;
*)
	echo "unknown archive format: $format" >&2
	exit 1
	;;
esac
rm -rf "$staging"
echo "dist/$name.$format"
