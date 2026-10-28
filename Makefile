.DEFAULT_GOAL := help

PYTHON ?= python3
CRATE ?=
TEST ?=
TEST_ARGS ?=
OUT ?= target/screenshots
TARGET_DIR = $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)

ifeq ($(OS),Windows_NT)
EXE := .exe
FORMATS ?= nsis
else
EXE :=
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
FORMATS ?= app,dmg
else ifeq ($(UNAME_S),Linux)
FORMATS ?= deb,appimage
endif
endif

.PHONY: help build release run run-release test fmt fmt-check lint check coverage screenshots smoke package clean

help: ## Show targets and common overrides
	@awk 'BEGIN { FS = ":.*## "; printf "Usage: make <target> [VARIABLE=value]\n\n" } /^[a-zA-Z_-]+:.*## / { printf "  %-16s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf '\nExamples:\n  make test CRATE=imposter-http TEST=redirect\n  make test TEST_ARGS="--nocapture"\n  make screenshots OUT=/tmp/imposter-screenshots\n  make smoke BINARY="target/packages/Imposter.app/Contents/MacOS/imposter"\n  make package FORMATS=app\n'

build: ## Build the debug application
	cargo build --locked -p imposter

release: ## Build the optimized application
	cargo build --locked --release -p imposter

run: ## Run the debug application
	cargo run --locked -p imposter

run-release: ## Run the optimized application
	cargo run --locked --release -p imposter

test: ## Run tests; optionally set CRATE, TEST (name filter), and TEST_ARGS
	cargo test --locked $(if $(CRATE),-p $(CRATE),--workspace) $(TEST) $(if $(TEST_ARGS),-- $(TEST_ARGS))

fmt: ## Format all Rust code
	cargo fmt --all

fmt-check: ## Check formatting without changing files
	cargo fmt --all --check

lint: ## Run Clippy on every target, treating warnings as errors
	cargo clippy --workspace --all-targets --locked -- -D warnings

check: ## Run the full gate: formatting, Clippy, tests, and coverage floors
	./scripts/check.sh

coverage: check ## Run the full gate including its coverage report

screenshots: ## Render the real UI offscreen to OUT (default: target/screenshots)
	cargo run --locked -p imposter-ui --example screenshot -- "$(OUT)"

smoke: ## Smoke-test the debug app, or an existing executable supplied as BINARY
ifdef BINARY
	$(PYTHON) scripts/smoke.py "$(BINARY)"
else
smoke: build
	$(PYTHON) scripts/smoke.py "$(TARGET_DIR)/debug/imposter$(EXE)"
endif

package: ## Build local packages; FORMATS defaults to the host OS formats
	@test -n "$(FORMATS)" || { printf '%s\n' 'Set FORMATS to the cargo-packager formats for your platform.'; exit 1; }
	CI=true cargo packager -p imposter --release --formats "$(FORMATS)"

clean: ## Remove Cargo build artifacts (keeps application data)
	cargo clean
