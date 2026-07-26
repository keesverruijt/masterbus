# masterbus — top-level Makefile
#
# Convenience wrappers over `cargo` for the most common workflows. Nothing
# here is required: `cargo build --release`, `cargo test --workspace`, etc.
# do the same job directly. The value is remembering the right invocation
# in one place, and having a `precommit` target that mirrors what CI checks.
#
# Common targets (see `make help` for the full list):
#   make               - Release build of every workspace member
#   make debug         - Debug build of every workspace member
#   make test          - Run the workspace test suite
#   make fmt           - `cargo fmt --all`
#   make clippy        - Workspace clippy at `-D warnings` (what CI enforces)
#   make precommit     - fmt-check + clippy + test — run this before pushing
#   make tools         - Release build of just the command-line tools
#   make publish       - Publish masterbus + masterbus-tools to crates.io
#   make clean         - `cargo clean`
#
# Per-developer targets (SSH deploys to your own boxes, regenerating the
# bundled string catalog from the reverse-engineering tree, scratch
# experiments) belong in `Makefile.local`, which is gitignored and
# `-include`d at the bottom of this file.

CARGO ?= cargo

.PHONY: all build debug check test fmt fmt-check clippy precommit \
        tools publish-dry publish \
        clean help

all: build

# Full-workspace release build. Everything published lands here.
build:
	$(CARGO) build --release --workspace

# Full-workspace debug build. Faster to compile, slower to run — useful
# for iterating on tests that link into a downstream binary.
debug:
	$(CARGO) build --workspace

# Quick type-check without producing binaries.
check:
	$(CARGO) check --workspace --all-targets

# Workspace test suite. `--workspace` covers the core crate, the tools,
# and the FFI demos.
test:
	$(CARGO) test --workspace

# Reformat the whole tree.
fmt:
	$(CARGO) fmt --all

# fmt but read-only (fails when files aren't formatted). What the CI
# `rustfmt` job runs.
fmt-check:
	$(CARGO) fmt --all --check

# Workspace-wide clippy at the same strictness CI enforces. If this passes
# locally your PR won't get bounced on lints.
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

# Everything you'd want green before opening a PR — mirrors CI (rustfmt +
# clippy + tests). Uses fmt-check (read-only); run `make fmt` to fix.
precommit: fmt-check clippy test

# Release build of just the three command-line tools (masterbus-tui,
# masterbus-signalk, masterbus-set-field) — handy when you don't need the
# core crate's tests or the FFI demos.
tools:
	$(CARGO) build --release -p masterbus-tools

# --- crates.io publishing ---------------------------------------------------
#
# The publishable crates, in dependency order. masterbus-ffi is `publish =
# false` (C-ABI demos, not a library) and is intentionally excluded. Bump
# the workspace version and commit before publishing — cargo refuses a dirty
# tree and refuses a version that already exists on crates.io.

# Dry-run the core crate: builds the packaged manifest exactly as crates.io
# will, without uploading. Run this first. (masterbus-tools can only be
# verified once the core crate it depends on is live, so it isn't dry-run
# here — its real publish below verifies against the just-published core.)
publish-dry:
	$(CARGO) publish -p masterbus --dry-run

# Publish to crates.io, core before tools. IRREVERSIBLE — a published
# version can only be yanked, never replaced or deleted. cargo waits for the
# core crate to land in the registry index before publishing the tools crate
# that depends on it, so no manual delay is needed. First publish of a crate
# name creates the package; later runs release new versions.
publish: publish-dry
	$(CARGO) publish -p masterbus
	$(CARGO) publish -p masterbus-tools

clean:
	$(CARGO) clean

help:
	@echo "masterbus Makefile targets:"
	@echo ""
	@echo "  make                Release build of every workspace member"
	@echo "  make debug          Debug build of every workspace member"
	@echo "  make check          Type-check without producing binaries"
	@echo "  make test           Run the workspace test suite"
	@echo "  make fmt            cargo fmt --all"
	@echo "  make fmt-check      cargo fmt --all --check (CI shape)"
	@echo "  make clippy         Workspace clippy at -D warnings (CI shape)"
	@echo "  make precommit      fmt-check + clippy + test (mirrors CI)"
	@echo ""
	@echo "  make tools          Release build of just the command-line tools"
	@echo ""
	@echo "  make publish-dry    Dry-run the crates.io package (core crate)"
	@echo "  make publish        Publish masterbus + masterbus-tools to crates.io"
	@echo ""
	@echo "  make clean          cargo clean"
	@echo ""
	@echo "Per-developer targets live in Makefile.local (gitignored)."

# Optional per-developer extensions — cross-compile deploys, hardware
# rigs, catalog regeneration, scratch experiments. The leading dash makes
# the include silent when the file is absent, so a fresh clone behaves
# identically to having no file. See the top-level comment for the split's
# rationale.
-include Makefile.local
