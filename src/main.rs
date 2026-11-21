//! anchor — file integrity monitor CLI.

use anchor::{
    diff, hash_path, load_baseline, post_webhook, save_baseline, scan, webhook_payload,
    Report,
};
use std::env;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        process::exit(1);
    }
    match args[0].as_str() {
        "init" | "baseline" => cmd_init(&args[1..]),
        "check" => cmd_check(&args[1..]),
        "watch" => cmd_watch(&args[1..]),
        "hash" => cmd_hash(&args[1..]),
        "-h" | "--help" | "help" => usage(),
        other => {
            eprintln!("unknown command: {}", other);
            usage();
            process::exit(1);
        }
    }
}

fn default_ignores() -> Vec<&'static str> {
    vec![".git", ".anchor", "target", "node_modules", ".DS_Store", "Cargo.lock"]
}

struct Flags {
    root: PathBuf,
    baseline: PathBuf,
    webhook: Option<String>,
    show_diff: bool,
    interval_ms: u64,
}

fn parse_flags(args: &[String], defaults_root: &str, defaults_base: &str) -> Flags {
    let mut root = PathBuf::from(defaults_root);
    let mut baseline = PathBuf::from(defaults_base);
    let mut webhook = None;
    let mut show_diff = false;
    let mut interval_ms = 1000u64;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--webhook" => {
                i += 1;
                if i < args.len() {
                    webhook = Some(args[i].clone());
                }
            }
            "--diff" => show_diff = true,
            "--interval" => {
                i += 1;
                if i < args.len() {
                    interval_ms = args[i].parse().unwrap_or(1000);
                }
            }
            "-h" | "--help" => {
                usage();
                process::exit(0);
            }
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {}", s);
                process::exit(1);
            }
            s => positional.push(s.to_string()),
        }
        i += 1;
    }
    if let Some(r) = positional.first() {
        root = PathBuf::from(r);
    }
    if let Some(b) = positional.get(1) {
        baseline = PathBuf::from(b);
    }
    Flags { root, baseline, webhook, show_diff, interval_ms }
}

fn cmd_init(args: &[String]) {
    let f = parse_flags(args, ".", ".anchor/baseline");
    let ignores = default_ignores();
    eprintln!("scanning {} …", f.root.display());
    match scan(&f.root, &ignores) {
        Ok(base) => {
            if let Some(parent) = f.baseline.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = save_baseline(&base, &f.baseline) {
                eprintln!("write failed: {}", e);
                process::exit(2);
            }
            println!("baseline: {} files → {}", base.entries.len(), f.baseline.display());
        }
        Err(e) => {
            eprintln!("scan failed: {}", e);
            process::exit(2);
        }
    }
}

fn cmd_check(args: &[String]) {
    let f = parse_flags(args, ".", ".anchor/baseline");
    let ignores = default_ignores();
    let baseline = match load_baseline(&f.baseline) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("load baseline: {}", e);
            process::exit(2);
        }
    };
    eprintln!("checking {} against {} …", f.root.display(), f.baseline.display());
    let live = match scan(&f.root, &ignores) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("scan failed: {}", e);
            process::exit(2);
        }
    };
    let report = diff(&baseline, &live, &f.root, f.show_diff);
    print_report(&report, f.show_diff);
    if !report.is_clean() {
        if let Some(ref url) = f.webhook {
            let body = webhook_payload(&report, &f.root.display().to_string());
            if let Err(e) = post_webhook(url, &body) {
                eprintln!("webhook failed: {}", e);
            } else {
                eprintln!("webhook sent → {}", url);
            }
        }
    }
    if report.is_clean() {
        println!("ok — {} files unchanged", report.unchanged);
        process::exit(0);
    } else {
        println!("FAIL — {} issue(s), {} unchanged", report.total_issues(), report.unchanged);
        process::exit(1);
    }
}

fn cmd_watch(args: &[String]) {
    let f = parse_flags(args, ".", ".anchor/baseline");
    let ignores = default_ignores();
    let mut baseline = match load_baseline(&f.baseline) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("no baseline — creating {}", f.baseline.display());
            let b = scan(&f.root, &ignores).unwrap_or_else(|e| {
                eprintln!("scan failed: {}", e);
                process::exit(2);
            });
            if let Some(parent) = f.baseline.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = save_baseline(&b, &f.baseline);
            b
        }
    };
    eprintln!(
        "watching {} every {}ms (baseline {}, {} files)",
        f.root.display(), f.interval_ms, f.baseline.display(), baseline.entries.len()
    );
    if let Some(ref url) = f.webhook {
        eprintln!("webhook: {}", url);
    }
    eprintln!("press Ctrl+C to stop\n");
    loop {
        thread::sleep(Duration::from_millis(f.interval_ms));
        let live = match scan(&f.root, &ignores) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("scan error: {}", e);
                continue;
            }
        };
        let report = diff(&baseline, &live, &f.root, f.show_diff);
        if report.is_clean() {
            continue;
        }
        for c in &report.changes {
            print!("[{}] {} {}", stamp(), c.kind.marker(), c.path);
            if f.show_diff {
                if let (Some(o), Some(n)) = (&c.old_hash, &c.new_hash) {
                    print!("  {}→{}", &o[..12.min(o.len())], &n[..12.min(n.len())]);
                }
                if let (Some(om), Some(nm)) = (c.old_mode, c.new_mode) {
                    if om != nm {
                        print!("  mode {:o}→{:o}", om, nm);
                    }
                }
            }
            println!();
            if f.show_diff {
                if let Some(ref d) = c.content_diff {
                    for line in d.lines().take(20) {
                        println!("      {}", line);
                    }
                }
            }
        }
        baseline = live;
        let _ = save_baseline(&baseline, &f.baseline);
        if let Some(ref url) = f.webhook {
            let body = webhook_payload(&report, &f.root.display().to_string());
            if let Err(e) = post_webhook(url, &body) {
                eprintln!("webhook failed: {}", e);
            }
        }
    }
}

fn stamp() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn cmd_hash(args: &[String]) {
    let path = match args.first() {
        Some(p) => p.as_str(),
        None => {
            eprintln!("usage: anchor hash <file>");
            process::exit(1);
        }
    };
    match hash_path(Path::new(path)) {
        Ok(e) => println!("{}", e.hex_hash()),
        Err(e) => {
            eprintln!("{}", e);
            process::exit(2);
        }
    }
}

fn print_report(r: &Report, show_diff: bool) {
    for c in &r.changes {
        println!("  {} {}", c.kind.marker(), c.path);
        if show_diff {
            if let (Some(o), Some(n)) = (&c.old_hash, &c.new_hash) {
                println!("      hash {} → {}", &o[..12.min(o.len())], &n[..12.min(n.len())]);
            }
            if let (Some(om), Some(nm)) = (c.old_mode, c.new_mode) {
                if om != nm {
                    println!("      mode {:o} → {:o}", om, nm);
                }
            }
            if let Some(ref d) = c.content_diff {
                for line in d.lines() {
                    println!("      {}", line);
                }
            }
        }
    }
}

fn usage() {
    eprintln!(
        "\x1b[1manchor\x1b[0m — file integrity monitor\n\n\
         \x1b[1mUsage:\x1b[0m\n\
           anchor init  [dir] [baseline]   create baseline\n\
           anchor check [dir] [baseline]   compare live tree to baseline\n\
           anchor watch [dir] [baseline]   live poll daemon\n\
           anchor hash  <file>             print SHA-256\n\n\
         \x1b[1mFlags:\x1b[0m\n\
           --diff                 show hash / mode / content hints\n\
           --webhook <url>        POST JSON alert\n\
           --interval <ms>        watch poll interval (default 1000)\n\n\
         Exit: 0 clean · 1 drift · 2 error\n\
         Zero dependencies. Pure Rust."
    );
}
