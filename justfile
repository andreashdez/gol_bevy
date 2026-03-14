fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features --locked -- -D warnings

check:
	cargo check --all-targets --locked

test:
	cargo test --all-targets --all-features --locked

readme-check:
	cargo test readme_contains_controls_text --locked

ci-local:
	just fmt-check
	just clippy
	just test
