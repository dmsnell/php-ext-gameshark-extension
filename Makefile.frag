GAMESHARK_CARGO_FLAGS ?= --locked

.PHONY: $(builddir)/rust/target/release/libgameshark_core.a $(builddir)/rust/target/release/libgameshark_core.dylib

$(builddir)/rust/target/release/libgameshark_core.a: $(srcdir)/rust/Cargo.toml $(srcdir)/rust/Cargo.lock $(srcdir)/rust/src/lib.rs
	mkdir -p $(builddir)/rust/target
	CARGO_TARGET_DIR=$(builddir)/rust/target $(CARGO) build --release $(GAMESHARK_CARGO_FLAGS) --manifest-path $(srcdir)/rust/Cargo.toml

$(builddir)/rust/target/release/libgameshark_core.dylib: $(srcdir)/rust/Cargo.toml $(srcdir)/rust/Cargo.lock $(srcdir)/rust/src/lib.rs
	mkdir -p $(builddir)/rust/target
	CARGO_TARGET_DIR=$(builddir)/rust/target $(CARGO) build --release $(GAMESHARK_CARGO_FLAGS) --manifest-path $(srcdir)/rust/Cargo.toml
	if test "$$(uname -s)" = "Darwin" && command -v install_name_tool >/dev/null 2>&1; then install_name_tool -id "@rpath/libgameshark_core.dylib" $(builddir)/rust/target/release/libgameshark_core.dylib; fi

.PHONY: gameshark-rust-clean
gameshark-rust-clean:
	CARGO_TARGET_DIR=$(builddir)/rust/target $(CARGO) clean --manifest-path $(srcdir)/rust/Cargo.toml
