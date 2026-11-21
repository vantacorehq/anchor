![anchor](banner.svg)

# anchor

File integrity monitor. Hash trees, detect tampering, baseline and diff. Pure Rust. Zero dependencies.

You point it at a directory once. It records size, mtime and SHA-256 for every regular file. Later you run `check`. Anything added, removed or changed is reported. Symlinks are not followed. Exit code 1 means drift.

## Why

Servers, containers, release artifacts, compliance packs — someone always asks “did this tree change?”. Anchor answers that question with one binary and no crates.

## Commands

```sh
# record a baseline
anchor init /etc /var/lib/anchor/etc.baseline

# later: did anything move?
anchor check /etc /var/lib/anchor/etc.baseline

# single file hash
anchor hash ./Cargo.toml
```

Defaults: `init` / `check` on `.`, baseline at `.anchor/baseline`.

Ignored by default: `.git`, `.anchor`, `target`, `node_modules`, `.DS_Store`, `Cargo.lock`.

## Output

```
  + new-file.conf
  - removed.conf
  ~ changed.conf
FAIL — 3 issue(s), 140 unchanged
```

```
ok — 143 files unchanged
```

## Rules

- Regular files only. Directories are walked; symlinks are skipped.
- Hash is SHA-256 (FIPS 180-4), implemented in-tree. No OpenSSL, no crates.
- Baseline format is plain text (`ANCHOR1`), one line per file, easy to audit and diff.
- Not a daemon. Run from cron, CI, or a release pipeline.

## Build

```sh
cargo test
cargo build --release
./target/release/anchor init .
./target/release/anchor check .
```

## License

MIT. Copyright 2026 vantacorehq.
