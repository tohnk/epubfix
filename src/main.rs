//! epubfix — repair common EPUB 2 validation errors in place.
//!
//! Drop the executable into a folder full of `.epub` files and run it. With no
//! arguments it scans the folder the executable itself lives in, fixes what it
//! can, and leaves everything else alone.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use epubfix::{Options, collect_epubs, fix_file, fixers};

const USAGE: &str = "\
epubfix — repair common EPUB 2 validation errors in place.

USAGE:
    epubfix [OPTIONS] [FILE_OR_DIR ...]

With no FILE_OR_DIR, every .epub in the folder containing the executable is
processed, so the tool can simply be dropped into a folder and run.

OPTIONS:
    -n, --dry-run       report what would change; write nothing
        --no-backup     do not keep a .bak copy of the original
    -r, --recursive     descend into subdirectories when scanning a folder
        --migrate-epub3 force an upgrade to EPUB 3 even if unnecessary
        --keep-version  never change a book's declared EPUB version
        --preserve-presentation
                        convert legacy table attributes to inline CSS
                        instead of removing them
        --only NAMES    run only these fixers (comma-separated, see --list)
    -l, --list          list the available fixers and exit
        --pause         wait for Enter before exiting
    -h, --help          show this help and exit
    -V, --version       show the version and exit
    --                  treat all remaining arguments as paths

A .bak copy is written beside each book that is modified, unless one already
exists — a second run never overwrites the pristine original.

Anything a fixer recognises as wrong but will not repair on its own is listed
under \"needs manual attention\" instead of being guessed at. Use --dry-run to
triage a whole library without writing to it.

VERSION RETAGGING:
    Books are often declared as the wrong EPUB version, in both directions. The
    declaration is one attribute; the content is thousands of elements. So when
    they disagree, epubfix moves the declaration to match the content rather
    than rewriting the content to match the declaration.

    Content that needs EPUB 3 (verse in <blockquote>, HTML5 elements, epub:
    attributes, a nav document) in a book declaring EPUB 2 -> upgraded, and the
    nav document, metadata and manifest properties EPUB 3 requires are
    generated. A book declaring EPUB 3 with none of that, written throughout as
    EPUB 2 -> downgraded, which is a single attribute plus a little package
    cleanup.

    Only decisive evidence retags. Mixed evidence leaves the declaration alone
    and repairs the book against whatever it currently claims. A retag that
    would lose a link, an id or any visible text is abandoned and the book left
    untouched. This all runs before every other fix, since the version decides
    what those fixes should do.

    --keep-version turns retagging off; --migrate-epub3 forces an upgrade even
    when the content does not require one.

EXIT STATUS:
    0  all good
    1  at least one book failed to process
    2  bad arguments
    3  everything worked, but some books need manual attention";

struct Args {
    opts: Options,
    paths: Vec<String>,
    recursive: bool,
    pause: Option<bool>,
}

fn parse_args(argv: Vec<String>) -> std::result::Result<Option<Args>, String> {
    let mut parsed = Args {
        opts: Options::default(),
        paths: Vec::new(),
        recursive: false,
        pause: None,
    };
    let mut it = argv.into_iter();
    let mut literal = false;

    while let Some(a) = it.next() {
        if literal || !a.starts_with('-') || a == "-" {
            parsed.paths.push(a);
            continue;
        }
        match a.as_str() {
            "--" => literal = true,
            "-n" | "--dry-run" => parsed.opts.dry_run = true,
            "--no-backup" => parsed.opts.backup = false,
            "-r" | "--recursive" => parsed.recursive = true,
            "--migrate-epub3" => parsed.opts.migrate_epub3 = true,
            "--keep-version" => parsed.opts.keep_version = true,
            "--preserve-presentation" => {
                parsed.opts.presentation = epubfix::Presentation::Preserve;
            }
            "--pause" => parsed.pause = Some(true),
            "--no-pause" => parsed.pause = Some(false),
            "-l" | "--list" => {
                list_fixers();
                return Ok(None);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("epubfix {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--only" => {
                let v = it.next().ok_or("--only needs a comma-separated list")?;
                parsed.opts.only.extend(split_names(&v));
            }
            _ => match a.strip_prefix("--only=") {
                Some(v) => parsed.opts.only.extend(split_names(v)),
                None => return Err(format!("unknown option `{a}` (try --help)")),
            },
        }
    }

    let known: Vec<&str> = fixers::all(&Options::default())
        .iter()
        .map(|f| f.name())
        .collect();
    if let Some(bad) = parsed
        .opts
        .only
        .iter()
        .find(|n| !known.contains(&n.as_str()))
    {
        return Err(format!("unknown fixer `{bad}` (try --list)"));
    }

    Ok(Some(parsed))
}

fn split_names(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn list_fixers() {
    let all = fixers::all(&Options::default());
    let width = all.iter().map(|f| f.name().len()).max().unwrap_or(0);
    println!("Available fixers:\n");
    for f in &all {
        println!(
            "  {:<width$}  {:<18}  {}",
            f.name(),
            f.codes().join(", "),
            f.description()
        );
    }
}

/// Where to look when the user gave no paths: the folder holding the executable,
/// falling back to the working directory if that cannot be determined.
fn default_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let args = match parse_args(env::args().skip(1).collect()) {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("epubfix: {e}");
            return ExitCode::from(2);
        }
    };

    let scanned = args.paths.is_empty();
    let mut targets: Vec<PathBuf> = Vec::new();
    if scanned {
        let dir = default_dir();
        println!("Scanning {} ...", dir.display());
        targets = collect_epubs(&dir, args.recursive);
    } else {
        for a in &args.paths {
            let p = PathBuf::from(a);
            if p.is_dir() {
                targets.extend(collect_epubs(&p, args.recursive));
            } else {
                targets.push(p);
            }
        }
    }

    if targets.is_empty() {
        println!("No .epub files found.");
    }

    let (mut fixed, mut clean, mut failed) = (0u32, 0u32, 0u32);
    let mut attention: Vec<(String, Vec<String>)> = Vec::new();
    for p in &targets {
        let name = p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        match fix_file(p, &args.opts) {
            Ok(outcome) => {
                if outcome.has_changes() {
                    let label = if args.opts.dry_run { " (dry run)" } else { "" };
                    println!("{name}:{label}\n    {}", outcome.changes.join("\n    "));
                    fixed += 1;
                } else {
                    println!("{name}: nothing to do");
                    clean += 1;
                }
                if !outcome.findings.is_empty() {
                    attention.push((name, outcome.findings));
                }
            }
            Err(e) => {
                eprintln!("{name}: FAILED ({e})");
                failed += 1;
            }
        }
    }

    if !attention.is_empty() {
        println!("\nNeeds manual attention:");
        for (name, findings) in &attention {
            println!("  {name}");
            for f in findings {
                println!("      {f}");
            }
        }
    }

    let verb = if args.opts.dry_run {
        "would fix"
    } else {
        "fixed"
    };
    print!("\nDone: {fixed} {verb}, {clean} already clean, {failed} failed");
    if attention.is_empty() {
        println!(".");
    } else {
        println!(", {} needing a look.", attention.len());
    }

    // Launched by double-click, the console closes the moment we return.
    let pause = args
        .pause
        .unwrap_or(scanned && cfg!(windows) && io::stdin().is_terminal());
    if pause {
        print!("Press Enter to exit...");
        io::stdout().flush().ok();
        io::stdin().read_line(&mut String::new()).ok();
    }

    // Distinct codes so a library sweep can be scripted: 1 means something
    // broke, 3 means everything worked but some books want a human.
    if failed > 0 {
        ExitCode::FAILURE
    } else if attention.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}
