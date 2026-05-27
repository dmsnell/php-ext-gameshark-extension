$(builddir)/rust/target/release/libgameshark_core.a: $(srcdir)/rust/Cargo.toml $(srcdir)/rust/src/lib.rs
	mkdir -p $(builddir)/rust/target
	CARGO_TARGET_DIR=$(builddir)/rust/target $(CARGO) build --release --manifest-path $(srcdir)/rust/Cargo.toml

.PHONY: gameshark-rust-clean
gameshark-rust-clean:
	CARGO_TARGET_DIR=$(builddir)/rust/target $(CARGO) clean --manifest-path $(srcdir)/rust/Cargo.toml
