set shell := ["bash", "-euo", "pipefail", "-c"]

binary := "target/release/yoink"
data_dir := env("HOME") / ".local/share/dev.garyj.yoink"
log := data_dir / "yoink.log"

# List recipes.
default:
    @just --list --unsorted

# Build the release binary.
build:
    cargo build --locked --release -p yoink

# Run Rust tests. Pass a name to run one test.
test *args:
    cargo test --locked --workspace {{ args }}

# Clippy and rustfmt check.
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo fmt --all --check

# Format Rust sources.
fmt:
    cargo fmt --all

# Everything CI would run: lint then test.
check: lint test

# Create release archives from committed sources.
package:
    scripts/package.sh

# Start the release binary detached (if already running, toggles its window instead).
start:
    @test -x {{ binary }} || { echo "no release binary, run: just build"; exit 1; }
    mkdir -p {{ data_dir }}
    setsid nohup env DISPLAY="${DISPLAY:-:0}" {{ binary }} >> {{ log }} 2>&1 < /dev/null &

# Stop the running instance.
stop:
    {{ binary }} --quit

# Stop, then start.
restart: stop
    sleep 1
    just start

# Follow the log.
log:
    tail -f {{ log }}
