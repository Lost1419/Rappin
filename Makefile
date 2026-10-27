.PHONY: check install-local voice-setup
FEATURES ?= cli,gateway,voice
PREFIX ?= $(HOME)/.local
check:
	cargo fmt --check
	cargo clippy --all-features --all-targets -- -D warnings
	cargo test --all-features --all-targets
	cargo test --all-features --doc
	cargo check --no-default-features
install-local:
	cargo build --release --features $(FEATURES)
	install -d $(PREFIX)/bin
	install -m 755 target/release/rappin $(PREFIX)/bin/rappin
voice-setup:
	sh scripts/setup-voice.sh
