# Dev Environment

This repo uses Nix flakes and direnv for a reproducible development shell.

## Prereqs

- Nix with flakes enabled
- direnv

## Enter the dev shell

1) Allow direnv in this repo:

```
direnv allow
```

2) Alternatively, enter the shell directly:

```
nix develop
```

The shell provides the Rust toolchain plus helpers for binary inspection and tracing.

## Build and run

```
cargo build
cargo run -- --help
```

## Basic workflow (stubs for now)

```
# Synthesize claims from the binary help output (--help with -h fallback)
cargo run -- claims --binary /usr/bin/ls --out ./claims.json

# Validate claims by executing the binary under controlled env constraints
cargo run -- validate --binary /usr/bin/ls --claims ./claims.json --out ./validation.json

# Render a man page view and a machine-readable report
cargo run -- regenerate --binary /usr/bin/ls --claims ./claims.json --results ./validation.json --out-man ./ls.1 --out-report ./report.json

```

## Notes

- The dev shell exports `RUST_BACKTRACE=1` for better diagnostics.
- `reports/` and `out/` are gitignored; `.gitkeep` keeps the directories in the repo.
