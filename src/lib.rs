//! Anchor — file integrity monitor.
//!
//! Walk a tree, hash every regular file, store a baseline, then re-check
//! and report added / removed / changed paths. Pure Rust. Zero dependencies.
//!
//! Hash is SHA-256 implemented from the FIPS 180-4 constants (no external crate).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::SystemTime;

/// One recorded file in a baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub mtime_secs: u64,
    pub sha256: [u8; 32],
}

impl Entry {
    pub fn hex_hash(&self) -> String {
        hex(&self.sha256)
    }
}

/// In-memory baseline: path → entry.
#[derive(Debug, Default, Clone)]
pub struct Baseline {
    pub entries: BTreeMap<String, Entry>,
    pub created_at: u64,
}

/// Diff between two baselines (or baseline vs live scan).
#[derive(Debug, Default)]
pub struct Report {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: usize,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    pub fn total_issues(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }
}

/// Walk `root`, hash every regular file, skip names matching `ignores`.
pub fn scan<P: AsRef<Path>>(root: P, ignores: &[&str]) -> io::Result<Baseline> {
    let root = root.as_ref();
    let mut entries = BTreeMap::new();
    walk(root, root, ignores, &mut entries)?;
    Ok(Baseline {
        entries,
        created_at: now_secs(),
    })
}

/// Compare `live` against `baseline`.
pub fn diff(baseline: &Baseline, live: &Baseline) -> Report {
    let mut report = Report::default();
    for (path, base) in &baseline.entries {
        match live.entries.get(path) {
            None => report.removed.push(path.clone()),
            Some(cur) => {
                if cur.sha256 != base.sha256 || cur.size != base.size {
                    report.changed.push(path.clone());
                } else {
                    report.unchanged += 1;
                }
            }
        }
    }
    for path in live.entries.keys() {
        if !baseline.entries.contains_key(path) {
            report.added.push(path.clone());
        }
    }
    report
}

/// Write baseline to a portable text file.
pub fn save_baseline<P: AsRef<Path>>(baseline: &Baseline, path: P) -> io::Result<()> {
    let mut f = File::create(path)?;
    writeln!(f, "ANCHOR1")?;
    writeln!(f, "{}", baseline.created_at)?;
    writeln!(f, "{}", baseline.entries.len())?;
    for (path, e) in &baseline.entries {
        writeln!(
            f,
            "{} {} {} {}",
            hex(&e.sha256),
            e.size,
            e.mtime_secs,
            path
        )?;
    }
    f.sync_all()?;
    Ok(())
}

/// Load a baseline written by `save_baseline`.
pub fn load_baseline<P: AsRef<Path>>(path: P) -> io::Result<Baseline> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let magic = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty baseline"))?;
    if magic != "ANCHOR1" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an anchor baseline",
        ));
    }
    let created_at: u64 = lines
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad created_at"))?;
    let count: usize = lines
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad count"))?;
    let mut entries = BTreeMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(4, ' ');
        let hash_hex = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing hash"))?;
        let size: u64 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad size"))?;
        let mtime_secs: u64 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad mtime"))?;
        let path = parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?
            .to_string();
        let sha256 = parse_hex32(hash_hex)?;
        entries.insert(
            path.clone(),
            Entry {
                path,
                size,
                mtime_secs,
                sha256,
            },
        );
    }
    let _ = count;
    Ok(Baseline {
        entries,
        created_at,
    })
}

fn walk(
    root: &Path,
    current: &Path,
    ignores: &[&str],
    out: &mut BTreeMap<String, Entry>,
) -> io::Result<()> {
    let meta = match fs::symlink_metadata(current) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        let rel = current
            .strip_prefix(root)
            .unwrap_or(current)
            .to_string_lossy()
            .replace('\\', "/");
        if should_ignore(&rel, ignores) {
            return Ok(());
        }
        let hash = hash_file(current)?;
        let mtime_secs = meta
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH)
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.insert(
            rel.clone(),
            Entry {
                path: rel,
                size: meta.len(),
                mtime_secs,
                sha256: hash,
            },
        );
        return Ok(());
    }
    if meta.is_dir() {
        let name = current
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if should_ignore(&name, ignores) {
            return Ok(());
        }
        let mut entries: Vec<_> = fs::read_dir(current)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            walk(root, &entry.path(), ignores, out)?;
        }
    }
    Ok(())
}

fn should_ignore(name: &str, ignores: &[&str]) -> bool {
    for pat in ignores {
        if pat.is_empty() {
            continue;
        }
        if name == *pat || name.ends_with(pat) || name.contains(pat) {
            return true;
        }
    }
    false
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn parse_hex32(s: &str) -> io::Result<[u8; 32]> {
    if s.len() != 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "hash must be 64 hex chars",
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = hex_val(s.as_bytes()[i * 2])?;
        let lo = hex_val(s.as_bytes()[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_val(c: u8) -> io::Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "bad hex")),
    }
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    bit_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: [0; 64],
            buffer_len: 0,
            bit_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.buffer[self.buffer_len] = b;
            self.buffer_len += 1;
            if self.buffer_len == 64 {
                self.compress();
                self.bit_len += 512;
                self.buffer_len = 0;
            }
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.bit_len + (self.buffer_len as u64) * 8;
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;
        if self.buffer_len > 56 {
            while self.buffer_len < 64 {
                self.buffer[self.buffer_len] = 0;
                self.buffer_len += 1;
            }
            self.compress();
            self.buffer_len = 0;
        }
        while self.buffer_len < 56 {
            self.buffer[self.buffer_len] = 0;
            self.buffer_len += 1;
        }
        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        self.compress();

        let mut out = [0u8; 32];
        for (i, &w) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    fn compress(&mut self) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                self.buffer[i * 4],
                self.buffer[i * 4 + 1],
                self.buffer[i * 4 + 2],
                self.buffer[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty() {
        let mut h = Sha256::new();
        h.update(b"");
        let dig = h.finalize();
        assert_eq!(
            hex(&dig),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        let mut h = Sha256::new();
        h.update(b"abc");
        let dig = h.finalize();
        assert_eq!(
            hex(&dig),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn scan_diff_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "anchor-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), b"hello").unwrap();
        fs::write(dir.join("sub/b.txt"), b"world").unwrap();

        let base = scan(&dir, &[]).unwrap();
        assert_eq!(base.entries.len(), 2);

        let path = dir.join("base.anchor");
        save_baseline(&base, &path).unwrap();
        let loaded = load_baseline(&path).unwrap();
        assert_eq!(loaded.entries.len(), 2);

        fs::write(dir.join("a.txt"), b"HELLO").unwrap();
        fs::write(dir.join("c.txt"), b"new").unwrap();
        fs::remove_file(dir.join("sub/b.txt")).unwrap();

        let live = scan(&dir, &[]).unwrap();
        let report = diff(&base, &live);
        assert!(report.changed.iter().any(|p| p.contains("a.txt")));
        assert!(report.removed.iter().any(|p| p.contains("b.txt")));
        assert!(report.added.iter().any(|p| p.contains("c.txt")));
        assert!(!report.is_clean());

        let _ = fs::remove_dir_all(&dir);
    }
}
