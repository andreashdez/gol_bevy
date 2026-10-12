fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features --locked -- -D warnings

check:
	cargo check --all-targets --locked

test:
	cargo test --all-targets --all-features --locked

bench:
	cargo bench --bench board_step --locked

readme-check:
	cargo test readme_contains_controls_text --locked

ci-local:
	just fmt-check
	just clippy
	just test

# Universal (Apple silicon + Intel) macOS binary in dist/. CI runs this for
# tagged releases; run it locally to test a release build. Needs a Mac.
release-macos version=`git describe --tags --always`:
	rustup target add aarch64-apple-darwin x86_64-apple-darwin
	CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked --target aarch64-apple-darwin
	CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked --target x86_64-apple-darwin
	mkdir -p target/universal-apple-darwin/release
	lipo -create -output target/universal-apple-darwin/release/gol-bevy target/aarch64-apple-darwin/release/gol-bevy target/x86_64-apple-darwin/release/gol-bevy
	scripts/package-release.sh "gol-bevy-{{version}}-universal-macos" target/universal-apple-darwin/release/gol-bevy
