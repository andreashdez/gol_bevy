#!/bin/sh
# Prints the release notes for a version: a line on how to run the download, then
# the version's section of CHANGELOG.md, without its heading.
#
#   scripts/release-notes.sh <version>
#
# Fails if CHANGELOG.md has no section for the version, or the section is empty.
# Used by the release job in .github/workflows/ci.yml and by
# scripts/check-release-version.sh.
set -eu

version=$1

# Everything after "## [<version>]" up to the next "## [" heading or link
# definition, with blank lines trimmed from both ends.
section=$(awk -v heading="## [$version]" '
	/^## \[/ || /^\[[^]]*\]: / { if (on) exit }
	index($0, heading) == 1 && (length($0) == length(heading) || substr($0, length(heading) + 1, 1) == " ") { on = 1; next }
	on { lines[++n] = $0 }
	END {
		first = 1; while (first <= n && lines[first] == "") first++
		last = n; while (last >= first && lines[last] == "") last--
		for (i = first; i <= last; i++) print lines[i]
	}
' CHANGELOG.md)

if [ -z "$section" ]; then
	echo "error: CHANGELOG.md has no notes under \"## [$version]\"; rename Unreleased to it" >&2
	exit 1
fi

printf 'Download the archive for your platform, unpack it, and run gol-bevy.\n\n%s\n' "$section"
