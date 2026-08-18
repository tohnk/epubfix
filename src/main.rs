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
        --keep-version  never change a book's declared EPUB version
        --preserve-presentation
                        convert every supported presentational attribute to an
                        inline style, even ones a stylesheet was overriding --
                        which promotes them above that stylesheet
        --strip-presentation
                        remove supported presentational attributes instead,
                        even ones holding up the layout; unsupported ones are
                        reported and kept
        --language-detect=MODE
                        when a missing <dc:language> may be written from
                        detected text: en-only (default), any, or off
        --keep-missing-images
                        report an <img> whose file is proven absent instead
                        of removing it and its now-empty wrapper
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
    attributes, a nav document, inline content inside <form> or <fieldset>) in
    a book declaring EPUB 2 -> upgraded, and the nav document, metadata and
    manifest properties EPUB 3 requires are generated. A book declaring EPUB 3
    with none of that, written throughout as EPUB 2 -> downgraded, which is a
    single attribute plus a little package cleanup.

    Only decisive evidence retags. A short run of inline content in a
    <blockquote> is not decisive — it is wrapped in a <div> and the book stays
    EPUB 2, which is the smaller change. Mixed evidence leaves the declaration
    alone and repairs the book against whatever it currently claims. A retag
    that would lose a link, an id or any visible text is abandoned and the book
    left untouched. This all runs before every other fix, since the version
    decides what those fixes should do.

    --keep-version turns retagging off, for a book that must keep the version
    it declares even when its content says otherwise.

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
            "--keep-version" => parsed.opts.keep_version = true,
            "--keep-missing-images" => parsed.opts.keep_missing_images = true,
            "--preserve-presentation" => {
                parsed.opts.presentation = epubfix::Presentation::Preserve;
            }
            "--strip-presentation" => {
                parsed.opts.presentation = epubfix::Presentation::Strip;
            }
            a if a.starts_with("--language-detect") => {
                let mode = a.strip_prefix("--language-detect=").unwrap_or("");
                parsed.opts.language = match mode {
                    "en-only" => epubfix::LanguagePolicy::EnglishOnly,
                    "any" => epubfix::LanguagePolicy::Any,
                    "off" => epubfix::LanguagePolicy::Off,
                    other => {
                        return Err(format!(
                            "--language-detect must be en-only, any or off (got \"{other}\")"
                        ));
                    }
                };
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
/// Report one book, and say whether it was changed.
///
/// What is still wrong is said right where the work is reported, so a long
/// sweep cannot leave the impression that a book came out in good order when
/// it did not.
fn needs_attention(outcome: &epubfix::Outcome) -> bool {
    !outcome.findings.is_empty() || !outcome.remaining.is_empty()
}

/// The summary's buckets, which have to hold two properties at once.
///
/// **Every book lands in exactly one.** The first version of this line
/// overlapped — a book could be both "fixed" and "needing a look" — and
/// printed `1 fixed, 0 nothing to do, 0 failed, 1 needing a look` for a single
/// book. It also had a gap: a book with nothing changed and problems remaining
/// matched neither arm and was counted nowhere.
///
/// **`fixed` means modified *and* fully repaired.** The overlap is resolved
/// by letting "needing a look" win: a book that was changed but still has
/// findings is counted there, not under "fixed". That is why the summary can
/// read `0 fixed, 1 needing a look` without contradiction — "fixed" never
/// claimed to count every modification, and "needing a look" makes no claim
/// about whether the book was touched. The per-book line above the summary
/// already said what happened to each book, which is where modifications are
/// reported.
#[derive(Default)]
struct Tally {
    fixed: u32,
    clean: u32,
    failed: u32,
    manual: u32,
}

impl Tally {
    fn record(&mut self, changed: bool, flagged: bool) {
        match (changed, flagged) {
            (true, false) => self.fixed += 1,
            (false, false) => self.clean += 1,
            // Changed or not, anything still wanting a person is one bucket.
            (_, true) => self.manual += 1,
        }
    }

    /// Only the tests ask this. It exists so the "every book lands in exactly
    /// one bucket" invariant is checkable rather than merely asserted in a
    /// comment — that is the property the first version of this line lacked.
    #[cfg(test)]
    fn total(&self) -> u32 {
        self.fixed + self.clean + self.failed + self.manual
    }

    /// `verb` is "fixed" or "would fix". The needing-a-look bucket is dropped
    /// when empty, which is the common case and keeps the usual line short.
    fn line(&self, verb: &str) -> String {
        let mut parts = vec![
            format!("{} {verb}", self.fixed),
            format!("{} nothing to do", self.clean),
            format!("{} failed", self.failed),
        ];
        if self.manual > 0 {
            parts.push(format!("{} needing a look", self.manual));
        }
        format!("Done: {}.", parts.join(", "))
    }
}

fn report_book(name: &str, outcome: &epubfix::Outcome, dry_run: bool) -> bool {
    let left = match outcome.remaining.len() {
        0 => String::new(),
        1 => "1 problem still remains".to_string(),
        n => format!("{n} problems still remain"),
    };
    let flagged = needs_attention(outcome);
    if outcome.has_changes() {
        let label = if dry_run { " (dry run)" } else { "" };
        println!("{name}:{label}\n    {}", outcome.changes.join("\n    "));
        if !left.is_empty() {
            println!("    ...but {left}.");
        }
        if flagged {
            // The summary counts this book under "needing a look", so the
            // connection has to be made here: the book was changed, and it is
            // not done yet.
            println!("    ...and it needs a look.");
        }
        return true;
    }
    if flagged {
        if left.is_empty() {
            println!("{name}: needs manual attention");
        } else {
            println!("{name}: needs manual attention; {left}.");
        }
    } else {
        println!("{name}: nothing to do");
    }
    false
}

/// Print a per-book list, skipping books with nothing in it.
fn report_section(heading: &str, entries: &[(String, Vec<String>)]) {
    if entries.iter().all(|(_, lines)| lines.is_empty()) {
        return;
    }
    println!("\n{heading}");
    for (name, lines) in entries.iter().filter(|(_, l)| !l.is_empty()) {
        println!("  {name}");
        for line in lines {
            println!("      {line}");
        }
    }
}

/// True if one of `findings` is already about the same file as `residual`.
///
/// The final scan and the fixers overlap by design — `dangling-resources`
/// declines to delete an `<img>` and reports it, and the scan then sees the
/// link it left behind. Both are right; printing both is the same defect twice.
/// Matching on the last path segment is enough, since the two describe it
/// differently ("images/logo.jpg" against "OEBPS/images/logo.jpg").
///
/// They can also differ in *spelling*, which is what made a *Rise of the Horde*
/// look worse than two books with the identical defect. A fixer quotes the href
/// as the document writes it, `%EF%BF%BD%EF%BF%BD`; the scan quotes where it
/// resolves to, which has been through the percent-decoder and reads `��`. The
/// same reference, and no substring shared between them. So both sides are
/// decoded before they are compared: the caller passes findings already
/// percent-decoded (once per book, not once per comparison), and the residual's
/// quoted target is decoded here.
fn already_explained(residual: &str, findings_decoded: &[String]) -> bool {
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    residual
        .split('"')
        .nth(1)
        .map(|target| decode(target.rsplit('/').next().unwrap_or(target)))
        .is_some_and(|leaf| {
            !leaf.is_empty() && findings_decoded.iter().any(|f| f.contains(leaf.as_str()))
        })
}

fn default_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Repair one book, turning a crash into an ordinary failure.
///
/// A bug that panics on one book must not take the rest of a library sweep with
/// it, which is what happened on a real one: a comment inside `<metadata>` was
/// enough to abort a whole run. Every repair happens in memory and the archive
/// is written only at the end, through a temp file and an atomic replace, so a
/// panic leaves the book exactly as it was found — all this has to clean up is
/// a temp file that never got moved into place.
///
/// This is why the release profile does not set `panic = "abort"`.
fn fix_file_guarded(path: &Path, opts: &Options) -> Result<epubfix::Outcome, String> {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fix_file(path, opts)));
    match attempt {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            let mut tmp = path.as_os_str().to_owned();
            tmp.push(".epubfix.tmp");
            let _ = std::fs::remove_file(PathBuf::from(tmp));
            Err(
                "internal error — see the panic above; the book was not modified, and this is \
                 a bug worth reporting"
                    .to_string(),
            )
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the CLI pipeline is intentionally visible in one top-level function"
)]
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

    let mut tally = Tally::default();
    let mut attention: Vec<(String, Vec<String>)> = Vec::new();
    let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
    for p in &targets {
        let name = p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        match fix_file_guarded(p, &args.opts) {
            Ok(outcome) => {
                let changed = report_book(&name, &outcome, args.opts.dry_run);
                tally.record(changed, needs_attention(&outcome));
                // A residual whose subject a finding already named is the same
                // defect said twice, so only the unexplained ones are listed.
                let decoded_findings: Vec<String> = outcome
                    .findings
                    .iter()
                    .map(|f| {
                        percent_encoding::percent_decode_str(f)
                            .decode_utf8_lossy()
                            .into_owned()
                    })
                    .collect();
                let unexplained: Vec<String> = outcome
                    .remaining
                    .iter()
                    .filter(|r| !already_explained(r, &decoded_findings))
                    .cloned()
                    .collect();
                if !outcome.findings.is_empty() {
                    attention.push((name.clone(), outcome.findings));
                }
                if !outcome.remaining.is_empty() {
                    unresolved.push((name, unexplained));
                }
            }
            Err(e) => {
                eprintln!("{name}: FAILED ({e})");
                tally.failed += 1;
            }
        }
    }

    report_section("Needs manual attention:", &attention);
    report_section(
        "Still wrong afterwards, and nothing above explains it:",
        &unresolved,
    );

    let verb = if args.opts.dry_run {
        "would fix"
    } else {
        "fixed"
    };
    println!("\n{}", tally.line(verb));
    if !unresolved.is_empty() {
        // Said once, at the end, because the counts above invite exactly the
        // wrong inference: epubfix looks for a handful of things, and a book it
        // has nothing to say about is not thereby a valid EPUB.
        println!("epubfix checks far less than EPUB Check does — run that for the real answer.");
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
    if tally.failed > 0 {
        ExitCode::FAILURE
    } else if tally.manual == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

#[cfg(test)]
mod tests {
    use super::{Tally, already_explained, needs_attention};
    use epubfix::Outcome;

    /// The property the first version of this line did not have: one book must
    /// contribute one to the totals. It printed `1 fixed, 0 nothing to do,
    /// 0 failed, 1 needing a look` for a single book, because a repaired book
    /// that still wanted a person was counted twice.
    #[test]
    fn every_book_lands_in_exactly_one_bucket() {
        let mut t = Tally::default();
        for (changed, flagged) in [(true, true), (true, false), (false, true), (false, false)] {
            t.record(changed, flagged);
        }
        t.failed += 1;
        assert_eq!(t.total(), 5);
    }

    /// The property the second version did not have: a book whose changes were
    /// just printed must never be counted under a bucket that implies it was
    /// untouched. "fixed" is only claimed for books with nothing left to flag,
    /// and "needing a look" makes no claim about modifications — so a changed
    /// book that still wants a person counts here, and the per-book line above
    /// the summary already said it was changed.
    #[test]
    fn a_changed_book_is_never_counted_as_nothing_to_do() {
        let mut t = Tally::default();
        t.record(true, true);
        let line = t.line("fixed");
        assert!(line.contains("1 needing a look"), "got {line}");
        assert!(
            !line.contains("1 nothing to do"),
            "a changed book is not untouched: {line}"
        );
        assert!(!line.contains("1 fixed,"), "not fully fixed either: {line}");
        assert_eq!(t.total(), 1);
    }

    /// The needing-a-look bucket is noise on a clean sweep, so it only appears
    /// when it has something to say.
    #[test]
    fn the_attention_bucket_is_dropped_when_empty() {
        let mut t = Tally::default();
        t.record(true, false);
        t.record(false, false);
        assert_eq!(t.line("fixed"), "Done: 1 fixed, 1 nothing to do, 0 failed.");
        assert_eq!(
            t.line("would fix"),
            "Done: 1 would fix, 1 nothing to do, 0 failed."
        );
    }

    #[test]
    fn findings_are_not_counted_as_nothing_to_do() {
        assert!(!needs_attention(&Outcome::none()));
        assert!(!needs_attention(&Outcome::change("changed")));
        assert!(needs_attention(&Outcome::finding("needs a caption")));
        let mut unresolved = Outcome::none();
        unresolved.remaining.push("still broken".to_string());
        assert!(needs_attention(&unresolved));
    }

    #[test]
    fn a_residual_and_a_finding_about_one_file_are_one_defect() {
        let findings_decoded = vec!["logo.jpg: 1 image(s) have no alt text".to_string()];
        assert!(already_explained(
            r#"ch1.xhtml: link to missing file "OEBPS/images/logo.jpg""#,
            &findings_decoded
        ));
        assert!(!already_explained(
            r#"ch1.xhtml: link to missing file "OEBPS/images/other.jpg""#,
            &findings_decoded
        ));
    }

    /// The two sides can describe the same reference in different spellings: a
    /// fixer quotes the href as written, the final scan quotes where it
    /// resolves to, which has been percent-decoded on the way. *Rise of the
    /// Horde* was listed twice for one defect because of it.
    #[test]
    fn the_same_reference_encoded_and_decoded_is_still_one_defect() {
        let findings_decoded = vec![
            percent_encoding::percent_decode_str(
                r#"part2.xhtml: <a> points at "%EF%BF%BD%EF%BF%BD", which is not in the book"#,
            )
            .decode_utf8_lossy()
            .into_owned(),
        ];
        assert!(already_explained(
            "part2.xhtml: link to missing file \"text/\u{FFFD}\u{FFFD}\"",
            &findings_decoded
        ));
    }

    /// A residual naming no file at all must not match everything.
    #[test]
    fn a_residual_with_no_quoted_target_explains_nothing() {
        let findings_decoded = vec!["something else entirely".to_string()];
        assert!(!already_explained(
            "no quoted target here",
            &findings_decoded
        ));
        assert!(!already_explained(r#"empty: """#, &findings_decoded));
    }
}
