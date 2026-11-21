//! anchor — file integrity monitor CLI.

use anchor::{diff, load_baseline, save_baseline, scan, Report};
use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        process::exit(1);
    }

    let cmd = args[0].as_str();
    match cmd {
        "init" | "baseline" => {
            let root = args.get(1).map(|s| s.as_str()).unwrap_or(".");
            let out = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or(".anchor/baseline");
            let ignores = default_ignores();
            eprintln!("scanning {} …", root);
            match scan(root, &ignores) {
                Ok(base) => {
                    if let Some(parent) = std::path::Path::new(out).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = save_baseline(&base, out) {
                        eprintln!("write failed: {}", e);
                        process::exit(2);
                    }
                    println!("baseline: {} files → {}", base.entries.len(), out);
                }
                Err(e) => {
                    eprintln!("scan failed: {}", e);
                    process::exit(2);
                }
            }
        }
        "check" => {
            let root = args.get(1).map(|s| s.as_str()).unwrap_or(".");
            let base_path = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or(".anchor/baseline");
            let ignores = default_ignores();
            let baseline = match load_baseline(base_path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("load baseline: {}", e);
                    process::exit(2);
                }
            };
            eprintln!("checking {} against {} …", root, base_path);
            let live = match scan(root, &ignores) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("scan failed: {}", e);
                    process::exit(2);
                }
            };
            let report = diff(&baseline, &live);
            print_report(&report);
            if report.is_clean() {
                println!("ok — {} files unchanged", report.unchanged);
                process::exit(0);
            } else {
                println!(
                    "FAIL — {} issue(s), {} unchanged",
                    report.total_issues(),
                    report.unchanged
                );
                process::exit(1);
            }
        }
        "hash" => {
            let path = match args.get(1) {
                Some(p) => p.as_str(),
                None => {
                    eprintln!("usage: anchor hash <file>");
                    process::exit(1);
                }
            };
            match hash_one(path) {
                Ok(h) => println!("{}", h),
                Err(e) => {
                    eprintln!("{}", e);
                    process::exit(2);
                }
            }
        }
        "-h" | "--help" | "help" => usage(),
        other => {
            eprintln!("unknown command: {}", other);
            usage();
            process::exit(1);
        }
    }
}

fn default_ignores() -> Vec<&'static str> {
    vec![
        ".git",
        ".anchor",
        "target",
        "node_modules",
        ".DS_Store",
        "Cargo.lock",
    ]
}

fn print_report(r: &Report) {
    for p in &r.added {
        println!("  + {}", p);
    }
    for p in &r.removed {
        println!("  - {}", p);
    }
    for p in &r.changed {
        println!("  ~ {}", p);
    }
}

fn hash_one(path: &str) -> std::io::Result<String> {
    use std::fs::File;
    use std::io::Read;
    let p = std::path::Path::new(path);
    if !p.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not a file",
        ));
    }
    let parent = p.parent().unwrap_or(std::path::Path::new("."));
    let name = p.file_name().unwrap().to_string_lossy();
    let base = scan(parent, &[])?;
    for (rel, e) in &base.entries {
        if rel == name.as_ref() || rel.ends_with(name.as_ref()) {
            return Ok(e.hex_hash());
        }
    }
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let dir = std::env::temp_dir().join(format!("anchor-hash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("f");
    std::fs::write(&tmp, &buf)?;
    let b = scan(&dir, &[])?;
    let h = b.entries.values().next().map(|e| e.hex_hash()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(h)
}

fn usage() {
    eprintln!(
        "anchor — file integrity monitor\n\n\
         Usage:\n\
           anchor init  [dir] [baseline]   create baseline (default: . / .anchor/baseline)\n\
           anchor check [dir] [baseline]   compare live tree to baseline\n\
           anchor hash  <file>             print SHA-256 of one file\n\n\
         Exit codes: 0 clean, 1 drift detected, 2 error\n\
         Zero dependencies. Pure Rust."
    );
}
