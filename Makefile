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
#   make release       - Bump version, tag, push (VERSION=X.Y.Z)
#   make publish       - Publish masterbus + masterbus-tools to crates.io
#   make clean         - `cargo clean`
#
# Per-developer targets (SSH deploys to your own boxes, regenerating the
# bundled string catalog from the reverse-engineering tree, scratch
# experiments) belong in `Makefile.local`, which is gitignored and
# `-include`d at the bottom of this file.

CARGO ?= cargo

.PHONY: all build debug check test fmt fmt-check clippy precommit \
        tools release publish-dry publish \
        clean help

DATE       := $(shell date +%Y-%m-%d)
OLDVERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

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

# --- release ----------------------------------------------------------------
#
# Cut a release:
#
#   make release VERSION=0.3.4
#
# Bumps the workspace version — both `[workspace.package] version` and the
# internal `masterbus` dependency pin — dates the CHANGELOG's [Unreleased]
# section (leaving a fresh empty one), verifies the tree (precommit),
# refreshes Cargo.lock, commits "Bump to X.Y.Z", tags vX.Y.Z, and pushes
# main + the tag. Pushing the tag triggers the CI release build (artifacts).
# Then `make publish` uploads to crates.io.
#
# precommit runs on the current tree *before* any edit, so a failing check
# leaves the working tree untouched. The bump only rewrites version strings
# and one CHANGELOG line, so it can't break a tree that was already green.
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=X.Y.Z"; exit 1; }
	@echo "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' \
	  || { echo "VERSION must be semver X.Y.Z (got '$(VERSION)')"; exit 1; }
	@test "$(VERSION)" != "$(OLDVERSION)" \
	  || { echo "already at $(VERSION)"; exit 1; }
	@git diff --quiet && git diff --cached --quiet \
	  || { echo "working tree not clean — commit or stash first"; exit 1; }
	$(MAKE) precommit
	perl -pi -e 's/"\Q$(OLDVERSION)\E"/"$(VERSION)"/g' Cargo.toml
	perl -pi -e 's/^## \[Unreleased\]\s*$$/## [Unreleased]\n\n## [$(VERSION)] - $(DATE)\n/' CHANGELOG.md
	$(CARGO) check --workspace   # refresh Cargo.lock to the new version
	git add Cargo.toml Cargo.lock CHANGELOG.md
	git commit -m "Bump to $(VERSION)"
	git tag v$(VERSION)
	git push origin main
	git push origin v$(VERSION)
	@echo "Released v$(VERSION). Run 'make publish' to upload to crates.io."

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
	@echo "  make release        VERSION=X.Y.Z — bump, changelog, tag, push"
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
