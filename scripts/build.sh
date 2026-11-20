#!/usr/bin/env bash
# Builds CAMCraft with cargo.
# Usage: ./build.sh [--debug] [--test] [--clean]
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$(dirname "$script_dir")"
manifest="$project/Cargo.toml"

profile="release"
run_test=0
clean=0

for arg in "$@"; do
    case "$arg" in
        --debug) profile="debug" ;;
        --test)  run_test=1 ;;
        --clean) clean=1 ;;
        -h|--help)
            echo "Usage: $0 [--debug] [--test] [--clean]"
            exit 0
            ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

command -v cargo >/dev/null 2>&1 || { echo "cargo not found. Install the Rust toolchain from https://rustup.rs" >&2; exit 1; }
[ -f "$manifest" ] || { echo "Cargo.toml not found at $manifest" >&2; exit 1; }

[ "$clean" -eq 1 ] && cargo clean --manifest-path "$manifest"
[ "$run_test" -eq 1 ] && cargo test --manifest-path "$manifest"

if [ "$profile" = "release" ]; then
    cargo build --manifest-path "$manifest" --release
else
    cargo build --manifest-path "$manifest"
fi

exe="$project/target/$profile/camcraft"
[ -f "$exe.exe" ] && exe="$exe.exe"
echo "Built: $exe"
