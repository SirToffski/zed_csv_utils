//! `csv-align` CLI fallback for Zed Tasks (no LSP needed).
//! Usage: `csv-align align|shrink <file.csv|file.tsv> [--in-place]`
//! Prints to stdout unless `--in-place` is given.

use std::env;
use std::fs;
use std::path::Path;

use csv_core::Dialect;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: csv-align align|shrink <file> [--in-place]");
        std::process::exit(2);
    }
    let mode = args[1].as_str();
    let file = &args[2];
    let in_place = args.iter().any(|a| a == "--in-place");
    let bytes = fs::read(file).unwrap_or_else(|e| {
        eprintln!("read {file}: {e}");
        std::process::exit(1);
    });
    // Samples may contain non-UTF8 bytes (latin-1); decode lossily like the
    // VS Code extension's binary/latin-1 handling.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let dialect = dialect_for_file(file);
    let out = match mode {
        "align" => csv_core::align_document(&text, dialect),
        "shrink" => csv_core::shrink_document(&text, dialect).0,
        _ => {
            eprintln!("unknown mode {mode:?}, want align|shrink");
            std::process::exit(2);
        }
    };
    if in_place {
        fs::write(file, out).unwrap_or_else(|e| {
            eprintln!("write {file}: {e}");
            std::process::exit(1);
        });
    } else {
        print!("{out}");
    }
}

fn dialect_for_file(file: &str) -> Dialect {
    let ext = Path::new(file).extension().and_then(|s| s.to_str()).unwrap_or("");
    Dialect::from_extension(ext).unwrap_or(Dialect::Csv)
}
