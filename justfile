fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features --locked -- -D warnings

# Lints the web build, which leaves out file dialogs and settings files.
clippy-web:
	rustup target add wasm32-unknown-unknown
	cargo clippy --target wasm32-unknown-unknown --locked -- -D warnings

check:
	cargo check --all-targets --locked

test:
	cargo test --all-targets --all-features --locked

bench:
	cargo bench --bench board_step --locked
	cargo bench --bench frame_loop --locked

readme-check:
	cargo test readme_lists_every_shortcut --locked

# Checks Cargo.lock against the RustSec advisories, as the Audit workflow does.
[doc("Checks Cargo.lock against the RustSec advisories. Needs cargo-deny.")]
audit:
	cargo deny --locked check advisories

ci-local:
	just fmt-check
	just clippy
	just clippy-web
	just test

# Builds the web version, as the docs site hosts it, into dist/web.
web:
	scripts/build-web.sh dist/web

# Builds the web version and serves it at http://localhost:8000.
web-serve: web
	python3 -m http.server --directory dist/web 8000

# Checks that a release tag matches the version in Cargo.toml, as CI does on a tag.
release-check tag:
	scripts/check-release-version.sh "{{tag}}"

# Tags HEAD for a release, once release-check passes. The message is
# "gol-bevy <tag>" and the version's notes from CHANGELOG.md; verbatim keeps
# the notes' `###` headings, which git would otherwise strip as comments. The
# tag is signed if git's tag.gpgSign is set. Push it yourself afterwards.
[doc("Tags HEAD for a release, with the version's notes from CHANGELOG.md as its message.")]
release-tag tag: (release-check tag)
	tag="{{tag}}"; { printf 'gol-bevy %s\n\n' "$tag"; scripts/release-notes.sh "${tag#v}"; } | git tag -a "$tag" --cleanup=verbatim -F -
	@echo "Tagged {{tag}}. Check it with: git show {{tag}}; then push it with: git push origin {{tag}}"

# Universal (Apple silicon + Intel) macOS binary in dist/. CI runs this for
# tagged releases; run it locally to test a release build. Needs a Mac.
[doc("Builds a universal macOS release binary into dist/. Needs a Mac.")]
release-macos version=`git describe --tags --always`:
	rustup target add aarch64-apple-darwin x86_64-apple-darwin
	CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked --target aarch64-apple-darwin
	CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked --target x86_64-apple-darwin
	mkdir -p target/universal-apple-darwin/release
	lipo -create -output target/universal-apple-darwin/release/gol-bevy target/aarch64-apple-darwin/release/gol-bevy target/x86_64-apple-darwin/release/gol-bevy
	scripts/package-release.sh "gol-bevy-{{version}}-universal-macos" target/universal-apple-darwin/release/gol-bevy
