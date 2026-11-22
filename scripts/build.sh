#!/usr/bin/env bash
# Builds CAMCraft with cargo.
# Usage: ./build.sh [--debug] [--test] [--clean] [--deb]
# --deb builds a release .deb in target/debian (needs: cargo install cargo-deb)
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$(dirname "$script_dir")"
manifest="$project/Cargo.toml"

profile="release"
run_test=0
clean=0
deb=0

for arg in "$@"; do
    case "$arg" in
        --debug) profile="debug" ;;
        --test)  run_test=1 ;;
        --clean) clean=1 ;;
        --deb)   deb=1 ;;
        -h|--help)
            echo "Usage: $0 [--debug] [--test] [--clean] [--deb]"
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

if [ "$deb" -eq 1 ]; then
    [ "$profile" = "release" ] || { echo "--deb builds release only, do not combine with --debug" >&2; exit 1; }
    command -v cargo-deb >/dev/null 2>&1 || { echo "cargo-deb not found. Run: cargo install cargo-deb" >&2; exit 1; }
    cargo deb --manifest-path "$manifest" --no-build
    echo "Built: $(ls "$project"/target/debian/*.deb)"
fi
