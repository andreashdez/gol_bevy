#!/bin/sh
# Packages a release binary with the README, changelog, licence, the bundled
# font's licence, and the notices of every crate compiled in, into dist/.
#
#   scripts/package-release.sh <archive name> <binary> [tar.gz|zip]
#
# Used by the release jobs in .github/workflows/ci.yml and by `just release-macos`, so
# every platform's download has the same layout. Needs cargo-about; see
# scripts/third-party-licenses.sh.
set -eu

name=$1
binary=$2
format=${3:-tar.gz}

staging=dist/$name
rm -rf "$staging"
mkdir -p "$staging"
cp "$binary" README.md CHANGELOG.md LICENSE assets/fonts/OFL.txt "$staging/"
scripts/third-party-licenses.sh "$staging/THIRD-PARTY-LICENSES.html" >/dev/null

case $format in
# COPYFILE_DISABLE keeps macOS's tar from adding a `._` file of extended
# attributes beside every file.
tar.gz) COPYFILE_DISABLE=1 tar -C dist -czf "dist/$name.tar.gz" "$name" ;;
zip) (cd dist && rm -f "$name.zip" && zip -qr "$name.zip" "$name") ;;
*)
	echo "unknown archive format: $format" >&2
	exit 1
	;;
esac
rm -rf "$staging"
echo "dist/$name.$format"
