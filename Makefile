# Local mirror of the checks run by .github/workflows/go.yml and
# .github/workflows/rust.yml. CI invokes these same targets (see the
# workflow files), so `make check` locally runs exactly what CI runs --
# there is only one copy of each underlying command to keep in sync.

.DEFAULT_GOAL := help

.PHONY: help
help:
	@echo "make check              run everything CI runs (go-check + rust-check)"
	@echo "make check-all          check + go-fuzz + rust-proptest-deep (slower)"
	@echo "make setup              one-time local tool install (staticcheck, ...)"
	@echo ""
	@echo "make go-check           go-vet + go-fmt + go-staticcheck + go-test"
	@echo "make go-vet             go vet ./..."
	@echo "make go-fmt             gofmt -l . (fails the build if it prints anything)"
	@echo "make go-staticcheck     staticcheck ./..."
	@echo "make go-test            go test ./... -timeout 60s"
	@echo ""
	@echo "make rust-check         rust-fmt + rust-clippy(+rand) + rust-test(+rand)"
	@echo "make rust-fmt           cargo fmt -- --check"
	@echo "make rust-clippy        cargo clippy --all-targets -- -D warnings"
	@echo "make rust-clippy-rand   cargo clippy --all-targets --features rand -- -D warnings"
	@echo "make rust-clippy-cli    cargo clippy --all-targets --features cli -- -D warnings"
	@echo "make rust-test          timeout 60 cargo test"
	@echo "make rust-test-rand     timeout 60 cargo test --features rand"
	@echo "make rust-test-cli      timeout 60 cargo test --features cli"
	@echo ""
	@echo "make go-fuzz            extended local fuzzing (30s), not part of check"
	@echo "make rust-proptest-deep 100k proptest cases, not part of check"

.PHONY: check
check: go-check rust-check

.PHONY: check-all
check-all: check go-fuzz rust-proptest-deep

.PHONY: setup
setup: go-tools
	@echo "Rust: make sure the rustfmt and clippy components are available for"
	@echo "your toolchain (e.g. 'rustup component add rustfmt clippy')."

.PHONY: go-tools
go-tools:
	cd go && go install honnef.co/go/tools/cmd/staticcheck@latest

## --- Go ---

# `go install`-ed tools (like staticcheck) land in $GOBIN, or GOPATH/bin if
# GOBIN is unset -- which may not be on the user's PATH. Prepending both
# candidates lets recipes find them regardless of shell config. Expanded at
# recipe run time (not `make` startup), so targets that don't use it never
# pay for the extra `go env` calls.
GOBIN_PATH = PATH="$$(go env GOBIN):$$(go env GOPATH)/bin:$$PATH"

.PHONY: go-check
go-check: go-vet go-fmt go-staticcheck go-test

.PHONY: go-vet
go-vet:
	cd go && go vet ./...

.PHONY: go-fmt
go-fmt:
	cd go && test -z "$$(gofmt -l .)"

.PHONY: go-staticcheck
go-staticcheck:
	cd go && $(GOBIN_PATH) staticcheck ./...

.PHONY: go-test
go-test:
	cd go && go test ./... -v -timeout 60s

## --- Rust ---

.PHONY: rust-check
rust-check: rust-fmt rust-clippy rust-clippy-rand rust-clippy-cli rust-test rust-test-rand rust-test-cli

.PHONY: rust-fmt
rust-fmt:
	cd rust && cargo fmt -- --check

.PHONY: rust-clippy
rust-clippy:
	cd rust && cargo clippy --all-targets -- -D warnings

.PHONY: rust-clippy-rand
rust-clippy-rand:
	cd rust && cargo clippy --all-targets --features rand -- -D warnings

.PHONY: rust-clippy-cli
rust-clippy-cli:
	cd rust && cargo clippy --all-targets --features cli -- -D warnings

.PHONY: rust-test
rust-test:
	cd rust && timeout 60 cargo test

.PHONY: rust-test-rand
rust-test-rand:
	cd rust && timeout 60 cargo test --features rand

.PHONY: rust-test-cli
rust-test-cli:
	cd rust && timeout 60 cargo test --features cli

## --- Extended, opt-in testing (not part of check/CI) ---

.PHONY: go-fuzz
go-fuzz:
	cd go && go test -fuzz=FuzzCompute -fuzztime=30s .

.PHONY: rust-proptest-deep
rust-proptest-deep:
	cd rust && PROPTEST_CASES=100000 cargo test --test properties
