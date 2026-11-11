---
recipe: 1
tools:
  - rust@1.95
  - node@22.18.0
setup:
  - cargo fetch --locked
  - npm --prefix web/hub ci --no-audit --no-fund
env:
  CARGO_INCREMENTAL: "0"
  CARGO_PROFILE_DEV_DEBUG: "0"
services: []
suggested_check: "env CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.hive/cache/cargo-target-land/amistio-template} CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 sh scripts/verify-check.sh"
---

# Environment

The toolchain Hive's own jobs run under, on the hub and on any rented hand
(ADR-026). `rust@1.95` is the workspace's `rust-version`; `node@22.18.0`
builds the hub console (`web/hub`), which `crates/hive-hub/build.rs` embeds
when `npm` is on PATH. Both pins are installed through mise by the job's
setup, so a freshly rented Linux machine is usable in about a minute (rust
took 22 s on a Standard_B2ms on 2026-11-05) instead of being marked unfit for
lacking `cargo`, released for idling, and rented again.

The suggested check names its shared Cargo target directory from `$HOME`
rather than an absolute path, so the same line builds under each machine's
own home.
