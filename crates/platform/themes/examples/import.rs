//! Imports a PyCharm UI theme, colour scheme, plugin archive or a directory of
//! them and prints what was mapped.
//!
//! ```text
//! cargo run -p themes --example import -- <file|jar|zip|dir> [options]
//!
//!   --context <path>   also search this file, archive or directory for parent
//!                      themes and schemes (repeatable); it is not imported
//!   --verbose          list every mapped, approximated and unmapped key
//!   --json             print the reports as JSON
//!   --toml <dir>       write each imported theme as <dir>/<id>.toml
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use themes::contrast::failures;
use themes::import::Library;

struct Args {
    input: PathBuf,
    context: Vec<PathBuf>,
    verbose: bool,
    json: bool,
    toml_dir: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut input = None;
    let mut context = Vec::new();
    let mut verbose = false;
    let mut json = false;
    let mut toml_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--context" => {
                context.push(PathBuf::from(args.next().ok_or("--context needs a path")?))
            }
            "--verbose" | "-v" => verbose = true,
            "--json" => json = true,
            "--toml" => {
                toml_dir = Some(PathBuf::from(
                    args.next().ok_or("--toml needs a directory")?,
                ))
            }
            "--help" | "-h" => return Err(String::new()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => {
                if input.replace(PathBuf::from(other)).is_some() {
                    return Err("only one input path is accepted".into());
                }
            }
        }
    }
    Ok(Args {
        input: input.ok_or("missing input path")?,
        context,
        verbose,
        json,
        toml_dir,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}");
            }
            eprintln!(
                "usage: import <file|jar|zip|dir> [--context <path>]... [--verbose] [--json] [--toml <dir>]"
            );
            return ExitCode::from(2);
        }
    };
    let mut library = Library::new();
    for extra in &args.context {
        if let Err(e) = library.add_path(extra, false) {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = library.add_path(&args.input, true) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let imported = library.import_all();
    if imported.is_empty() {
        eprintln!(
            "error: no UI theme or colour scheme found in {}",
            args.input.display()
        );
        return ExitCode::FAILURE;
    }

    if args.json {
        let reports: Vec<_> = imported.iter().map(|i| &i.report).collect();
        match serde_json::to_string_pretty(&reports) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        for archive in library.archives() {
            println!(
                "archive {}: {} entries, {} data files read, {} class files untouched, {} other entries untouched",
                archive.path,
                archive.entries,
                archive.read.len(),
                archive.ignored_classes,
                archive.ignored_other
            );
        }
        for skipped in library.skipped() {
            println!("skipped {}: {}", skipped.path, skipped.reason);
        }
        for item in &imported {
            print!("{}", item.report.render(args.verbose));
            let bad = failures(&item.theme);
            println!(
                "  contrast: {} of the checked pairs are below WCAG AA (informational)",
                bad.len()
            );
        }
        let total = imported.len();
        let s = imported.iter().fold((0, 0, 0), |acc, i| {
            let s = i.report.summary();
            (
                acc.0 + s.mapped,
                acc.1 + s.unmapped_keys,
                acc.2 + s.defaulted,
            )
        });
        println!(
            "{total} themes: {} mapped keys, {} unmapped keys, {} tokens from the base theme",
            s.0, s.1, s.2
        );
    }

    if let Some(dir) = &args.toml_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("error: {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
        for item in &imported {
            let path = dir.join(format!("{}.toml", item.theme.id));
            if let Err(e) = std::fs::write(&path, item.theme.to_file().to_toml()) {
                eprintln!("error: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
