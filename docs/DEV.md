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

The shell provides the Rust toolchain and bwrap.

## Build and run

```
cargo build
cargo run --bin bman -- ls
```

The configured LM CLI must be authenticated/configured to run. Evidence is written
under `out/evidence/<run_id>/`, where `<run_id>` is `<label>-<hash12>-<epoch_ms>`
and `label` is a slugged scenario ID (or an error code for early failures).
