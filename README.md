<p align="center">
  <img src="banner.svg" alt="anchor" width="100%">
</p>

<p align="center">
  <b>File integrity monitor</b> — baseline · live watch · webhooks · JSON<br>
  SHA-256 in-tree · parallel scan · zero dependencies
</p>

<p align="center">
  <img src="https://img.shields.io/badge/rust-stable-orange?style=flat-square" alt="rust">
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="license">
  <img src="https://img.shields.io/badge/deps-zero-brightgreen?style=flat-square" alt="deps">
</p>

---

## Why

Servers, containers, release trees, compliance packs — someone always asks *did this change?*  
**anchor** freezes a SHA-256 baseline and can sit as a daemon that prints (or webhooks) the moment anything moves.

## Install

```sh
cargo install --path .
# or
cargo build --release && ./target/release/anchor --help
```

## Commands

```sh
# Freeze a tree
anchor init /etc

# One-shot audit
anchor check /etc --diff

# Compact summary (CI-friendly)
anchor status /etc

# Machine-readable
anchor check /etc --json

# Live daemon
anchor watch /etc --diff --interval 500

# Discord / generic webhook
anchor watch /etc --webhook http://127.0.0.1:9000/hook

# Extra ignores (also reads `.anchorignore`)
anchor check /var/www --ignore '*.log' --ignore 'cache/'
```

### Flags

| Flag | Effect |
|------|--------|
| `--diff` | Hash / mode / content hints |
| `--json` | Structured report on stdout |
| `--quiet` / `-q` | Less stderr |
| `--ignore <pat>` | Extra ignore (repeatable) |
| `--webhook <url>` | POST JSON alert on drift |
| `--interval <ms>` | Watch poll interval (default 1000) |

Exit codes: `0` clean · `1` drift · `2` error.

## Ignore rules

Defaults: `.git`, `.anchor`, `target`, `node_modules`, `.DS_Store`, `Cargo.lock`, `*.anchor`.

Put a `.anchorignore` in the monitored root (see `.anchorignore.example`).

## What it tracks

- **SHA-256** of every regular file (FIPS 180-4, no OpenSSL)
- **Size**, **mtime**, **Unix mode** bits
- Symlinks are **not** followed
- Hashing is **parallel** across available CPUs

## Baseline format

Plain text `ANCHOR2`. Easy to audit. `ANCHOR1` still loads.

## CI

`.github/workflows/ci.yml` — fmt, clippy, test on Ubuntu / Windows / macOS, release binaries as artifacts.

## License

MIT · Copyright 2026 vantacorehq
