//! `sf-impact-extract`: reads the tree a Salesforce DX project keeps on disk and
//! writes its refs, one declaration or reference per row: as a refs pack
//! (`docs/pack.md`), or as `refs.jsonl`, one row per line. The rows are
//! defined by `schema/refs.schema.json`.

use sf_impact_common::diagnostic::said;
use sf_impact_common::files::{self, Replacement};
use sf_impact_common::pack::Pack;
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs, thread};

const USAGE: &str = "\
usage: sf-impact-extract --root <dir> [--out <file>] [--format <format>] [--jobs <n>]
                         [--trust-repo] [--previous <file>]
       sf-impact-extract --dump <pack>

Reads the package directories named in <dir>/sfdx-project.json and writes
its refs: what it read, then declarations and references. They are written
as a refs pack, which sf-impact-changed and sf-impact-tests read.

  --root <dir>   the Salesforce DX project to read
  --out <file>   where to write; standard output if absent or `-`
  --format <format>
                 `pack`, the default: a refs pack (schema 2, docs/pack.md).
                 `jsonl`: refs.jsonl, one JSON object per line, as --dump
                 writes the pack, to read or keep; the other stages do not
                 read it
  --jobs <n>     worker threads; by default one per CPU, at most 8
  --trust-repo   ask git which commit the tree is though another user owns
                 the repository, as in a container whose workspace someone
                 else made. Git refuses to on its own, and the tree is then
                 `unknown`. Only this repository is trusted, and nothing is
                 written to any configuration
  --previous <file>
                 the pack that an earlier run wrote to the same --out, to
                 reuse the rows of the files that have not changed since. It
                 must name the file that --out names, which is replaced in
                 place: there is only ever one pack. The pack written is the
                 one a run without --previous writes, byte for byte. A
                 previous pack that is missing, damaged or of anything else
                 (another release, scope, config or object format) is not
                 reused, and why is said on standard error

The first row says which commit the tree is, if all that was read is as the
commit that is checked out has it: git is asked, and nothing is written.

  --dump <pack>  write the rows of a refs pack (schema 2, docs/pack.md) to
                 standard output as refs.jsonl, line 1 first. Only <pack>
                 is read
";

/// Reading files dominates, so more workers than this gain nothing.
const MAX_DEFAULT_JOBS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Extract {
        root: PathBuf,
        out: Option<PathBuf>,
        format: Format,
        jobs: usize,
        trust_repo: bool,
        previous: Option<PathBuf>,
    },
    Dump(PathBuf),
    Help,
    Version,
}

/// What the refs are written as.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Format {
    Pack,
    Jsonl,
}

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

unsafe extern "C" fn ts_malloc(size: usize) -> *mut std::ffi::c_void {
    // SAFETY: forwards tree-sitter's allocation request to mimalloc.
    let ptr = unsafe { libmimalloc_sys::mi_malloc(size) };
    if ptr.is_null() && size != 0 {
        std::process::abort();
    }
    ptr
}

unsafe extern "C" fn ts_calloc(count: usize, size: usize) -> *mut std::ffi::c_void {
    // SAFETY: forwards tree-sitter's allocation request to mimalloc.
    let ptr = unsafe { libmimalloc_sys::mi_calloc(count, size) };
    if ptr.is_null() && count != 0 && size != 0 {
        std::process::abort();
    }
    ptr
}

unsafe extern "C" fn ts_realloc(ptr: *mut std::ffi::c_void, size: usize) -> *mut std::ffi::c_void {
    // SAFETY: tree-sitter passes a null pointer or an allocation from this family.
    let result = unsafe { libmimalloc_sys::mi_realloc(ptr, size) };
    if result.is_null() && size != 0 {
        std::process::abort();
    }
    result
}

fn configure_tree_sitter_allocator() {
    // SAFETY: called before any tree-sitter allocation and never reset. The
    // wrappers abort on null for nonzero requests, and mimalloc supplies malloc
    // alignment. All functions use the same allocator family; mi_free can free
    // allocations from any thread or mimalloc heap.
    unsafe {
        tree_sitter::set_allocator(Some(tree_sitter::Allocator {
            malloc: ts_malloc,
            calloc: ts_calloc,
            realloc: ts_realloc,
            free: libmimalloc_sys::mi_free,
        }));
    }
}

fn main() -> ExitCode {
    // Keep this first, before any tree-sitter parser exists.
    configure_tree_sitter_allocator();
    sf_impact_common::allocator::return_freed_memory_at_once();
    let command = match parse_args(env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprint!("sf-impact-extract: {}\n\n{USAGE}", said(&message));
            return ExitCode::from(2);
        }
    };
    let (root, out, format, jobs, trust_repo, previous) = match command {
        Command::Help => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("sf-impact-extract {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Dump(pack) => {
            let read = fs::read(&pack).map_err(|e| format!("{}: {e}", pack.display()));
            let rows = read.and_then(|bytes| Pack::decode(&bytes).map(|pack| pack.to_jsonl()));
            return match rows.and_then(|rows| write(None, false, lines(&rows))) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("sf-impact-extract: {}", said(&message));
                    ExitCode::FAILURE
                }
            };
        }
        Command::Extract {
            root,
            out,
            format,
            jobs,
            trust_repo,
            previous,
        } => (root, out, format, jobs, trust_repo, previous),
    };
    if let Some(previous) = &previous
        && let Err(message) = is_out(previous, out.as_deref(), format)
    {
        eprint!("sf-impact-extract: {}\n\n{USAGE}", said(&message));
        return ExitCode::from(2);
    }
    // A pack kept for the next run is made durable before the run ends.
    let keeps = previous.is_some();
    // Read whole before anything is written: the file is then replaced. A
    // pack that cannot be read is not reused, and that is all. It is the
    // file that --out names, and it is read by that name: only a regular
    // file there, not a link, is read, as only one is replaced.
    let read = previous
        .as_ref()
        .zip(out.as_deref())
        .map(|(_, out)| files::read_regular(out, u64::MAX));
    let previous = match read {
        Some(Err(e)) if e.kind() != io::ErrorKind::NotFound => {
            eprintln!(
                "sf-impact-extract: note: the previous pack was not reused: {}",
                said(&e.to_string())
            );
            None
        }
        Some(Err(_)) => {
            eprintln!("sf-impact-extract: note: the previous pack was not reused: there is none");
            None
        }
        Some(Ok(bytes)) => Some(bytes),
        None => None,
    };
    let pack = sf_impact_extract::extract_reusing(&root, jobs, trust_repo, previous.as_deref());
    let written = pack.and_then(|(pack, kept)| {
        let written = match format {
            Format::Pack => write(out.as_deref(), keeps, |sink| sink.write_all(&pack)),
            Format::Jsonl => Pack::decode(&pack).and_then(|decoded| {
                let rows = decoded.to_jsonl();
                let written = write(out.as_deref(), false, lines(&rows));
                std::mem::forget((decoded, rows));
                written
            }),
        };
        // Deliberate: the process is about to exit, and freeing the pack and
        // what resolved it one allocation at a time is a measurable share
        // of the run. The OS reclaims it all at once. Only after the write,
        // which flushes; the library's extract frees as usual.
        std::mem::forget((pack, kept, previous));
        written
    });
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sf-impact-extract: {}", said(&message));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let (mut root, mut out, mut jobs, mut dump) = (None, None, None, None);
    let mut format = None;
    let (mut trust_repo, mut previous) = (false, None);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(value()?)),
            "--out" => out = Some(value()?).filter(|out| out != "-").map(PathBuf::from),
            "--jobs" => {
                let value = value()?;
                jobs = match value.parse() {
                    Ok(jobs) if jobs > 0 => Some(jobs),
                    _ => return Err(format!("--jobs needs a positive number, not `{value}`")),
                };
            }
            "--format" => {
                format = match value()?.as_str() {
                    "pack" => Some(Format::Pack),
                    "jsonl" => Some(Format::Jsonl),
                    other => return Err(format!("--format is `pack` or `jsonl`, not `{other}`")),
                };
            }
            "--trust-repo" => trust_repo = true,
            "--previous" => previous = Some(PathBuf::from(value()?)),
            "--dump" => dump = Some(PathBuf::from(value()?)),
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if let Some(pack) = dump {
        let alone = root.is_none()
            && out.is_none()
            && format.is_none()
            && jobs.is_none()
            && !trust_repo
            && previous.is_none();
        return match alone {
            true => Ok(Command::Dump(pack)),
            false => Err("--dump takes no other option".into()),
        };
    }
    let cpus = thread::available_parallelism().map_or(1, |n| n.get());
    Ok(Command::Extract {
        root: root.ok_or("--root is required")?,
        out,
        format: format.unwrap_or(Format::Pack),
        jobs: jobs.unwrap_or(cpus.min(MAX_DEFAULT_JOBS)),
        trust_repo,
        previous,
    })
}

/// Whether the previous pack can be replaced by what is written: only a
/// pack replaces it, and only in the same file, so that there is never
/// more than one.
fn is_out(previous: &Path, out: Option<&Path>, format: Format) -> Result<(), String> {
    if format != Format::Pack {
        return Err("--previous is a pack, and --format jsonl does not write one".into());
    }
    match out {
        Some(out) if same_file(previous, out) => Ok(()),
        _ => Err("--previous must name the file that --out names".into()),
    }
}

/// Whether two paths name one file: where both exist, the same file on
/// the same device, by whatever names, links included; where neither does,
/// the same name in the same directory, however the directory is named.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
        (Err(_), Err(_)) => {
            let named = |path: &Path| {
                let name = path.file_name()?;
                let directory = path.parent().filter(|d| !d.as_os_str().is_empty());
                let directory = fs::canonicalize(directory.unwrap_or(Path::new("."))).ok()?;
                Some(directory.join(name))
            };
            named(a).is_some_and(|a| named(b) == Some(a))
        }
        _ => false,
    }
}

/// The lines of refs.jsonl, each with its line feed.
fn lines(lines: &[String]) -> impl Fn(&mut dyn Write) -> io::Result<()> {
    move |sink| {
        for line in lines {
            sink.write_all(line.as_bytes())?;
            sink.write_all(b"\n")?;
        }
        Ok(())
    }
}

/// Writes `content` to `out`, or to standard output. A file is written
/// beside `out` and renamed over it ([`Replacement`]), and with `sync` it
/// is on disk before this returns.
fn write(
    out: Option<&Path>,
    sync: bool,
    content: impl Fn(&mut dyn Write) -> io::Result<()>,
) -> Result<(), String> {
    let write_to = |sink: &mut dyn Write| {
        let mut sink = io::BufWriter::new(sink);
        content(&mut sink)?;
        sink.flush()
    };
    match out {
        Some(path) => Replacement::new(path)
            .and_then(|mut file| {
                write_to(&mut file)?;
                file.finish(sync)
            })
            .map_err(|e| format!("{}: {e}", path.display())),
        None => match write_to(&mut io::stdout().lock()) {
            // The reader stopped early (`| head`): not an error.
            Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(e.to_string()),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn root_out_and_jobs_are_read_from_the_command_line() {
        let command = parse(&["--root", "repo", "--out", "refs.jsonl", "--jobs", "3"]);

        assert_eq!(
            command,
            Ok(Command::Extract {
                root: "repo".into(),
                out: Some("refs.jsonl".into()),
                format: Format::Pack,
                jobs: 3,
                trust_repo: false,
                previous: None,
            })
        );
    }

    #[test]
    fn the_previous_pack_is_read_from_the_command_line() {
        let command = parse(&["--root", "r", "--previous", "p", "--out", "p"]);

        let Ok(Command::Extract { previous, .. }) = command else {
            panic!("should parse");
        };
        assert_eq!(previous, Some("p".into()));
        assert!(parse(&["--dump", "refs.pack", "--previous", "p"]).is_err());
    }

    #[test]
    fn the_previous_pack_is_replaced_only_by_a_pack_in_the_same_file() {
        let here = env::temp_dir();
        let pack = here.join("refs.pack");
        let again = here.join(".").join("refs.pack");

        assert_eq!(is_out(&pack, Some(&again), Format::Pack), Ok(()));
        assert!(is_out(&pack, Some(&here.join("other.pack")), Format::Pack).is_err());
        assert!(is_out(&pack, None, Format::Pack).is_err());
        assert!(is_out(&pack, Some(&pack), Format::Jsonl).is_err());
        // A file in no directory there is.
        assert!(!same_file(&here.join("nowhere/a"), &here.join("nowhere/a")));
    }

    #[test]
    fn a_repository_is_trusted_only_when_the_command_line_says_so() {
        let Ok(Command::Extract { trust_repo, .. }) = parse(&["--root", "repo", "--trust-repo"])
        else {
            panic!("should parse");
        };

        assert!(trust_repo);
    }

    #[test]
    fn output_goes_to_stdout_when_out_is_absent_or_a_dash() {
        for args in [&["--root", "repo"][..], &["--root", "repo", "--out", "-"]] {
            let Ok(Command::Extract { out, .. }) = parse(args) else {
                panic!("{args:?} should parse");
            };

            assert_eq!(out, None);
        }
    }

    #[test]
    fn the_default_worker_count_is_capped() {
        let Ok(Command::Extract { jobs, .. }) = parse(&["--root", "repo"]) else {
            panic!("should parse");
        };

        assert!((1..=MAX_DEFAULT_JOBS).contains(&jobs));
    }

    #[test]
    fn the_refs_are_a_pack_unless_the_command_line_says_jsonl() {
        let format = |args: &[&str]| match parse(args) {
            Ok(Command::Extract { format, .. }) => Ok(format),
            other => Err(format!("{other:?}")),
        };

        assert_eq!(format(&["--root", "repo"]), Ok(Format::Pack));
        assert_eq!(
            format(&["--root", "r", "--format", "pack"]),
            Ok(Format::Pack)
        );
        assert_eq!(
            format(&["--root", "r", "--format", "jsonl"]),
            Ok(Format::Jsonl)
        );
        assert!(format(&["--root", "r", "--format", "json"]).is_err());
        assert!(parse(&["--dump", "refs.pack", "--format", "jsonl"]).is_err());
    }

    #[test]
    fn dump_reads_one_pack_and_nothing_else() {
        assert_eq!(
            parse(&["--dump", "refs.pack"]),
            Ok(Command::Dump("refs.pack".into()))
        );
        assert!(parse(&["--dump", "refs.pack", "--root", "repo"]).is_err());
        assert!(parse(&["--dump"]).is_err());
    }

    #[test]
    fn bad_command_lines_are_refused_with_a_reason() {
        assert_eq!(parse(&[]), Err("--root is required".into()));
        assert_eq!(parse(&["--root"]), Err("--root needs a value".into()));
        assert!(parse(&["--root", "repo", "--jobs", "0"]).is_err());
        assert!(parse(&["--root", "repo", "--frobnicate"]).is_err());
    }
}

#[cfg(test)]
mod allocator_test {
    // This module swaps tree-sitter's allocator for the whole test binary;
    // no other test in this binary may create a parser concurrently.
    #[test]
    fn allocation_wrappers_accept_normal_and_zero_sizes() {
        // SAFETY: each returned allocation is freed once through mimalloc.
        unsafe {
            let p = super::ts_malloc(32);
            assert!(!p.is_null());
            let p = super::ts_realloc(p, 64);
            assert!(!p.is_null());
            libmimalloc_sys::mi_free(p);
            let p = super::ts_calloc(2, 32);
            assert!(!p.is_null());
            libmimalloc_sys::mi_free(p);
            libmimalloc_sys::mi_free(super::ts_malloc(0));
            libmimalloc_sys::mi_free(super::ts_calloc(0, 32));
            libmimalloc_sys::mi_free(super::ts_calloc(32, 0));
            libmimalloc_sys::mi_free(super::ts_realloc(std::ptr::null_mut(), 0));
        }
    }

    #[test]
    fn parser_is_allocated_by_mimalloc() {
        super::configure_tree_sitter_allocator();
        // SAFETY: the parser is live when inspected and is deleted exactly once.
        unsafe {
            let parser = tree_sitter::ffi::ts_parser_new();
            assert!(!parser.is_null());
            let owned = libmimalloc_sys::mi_is_in_heap_region(parser.cast());
            tree_sitter::ffi::ts_parser_delete(parser);
            assert!(owned, "tree-sitter parser was not allocated by mimalloc");
        }
    }
}
