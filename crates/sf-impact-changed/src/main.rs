//! `sf-impact-changed`: maps the difference between two commits onto the symbols
//! of a Salesforce DX project and writes `changed.jsonl`, one changed file,
//! changed symbol or touched name per line. The rows are defined by
//! `schema/changed.schema.json`.

use sf_impact_common::diagnostic::said;
use sf_impact_common::files::Replacement;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

const USAGE: &str = "\
usage: sf-impact-changed --delta <file> --refs <file> [--out <file>]
                   [--accept-unverified-refs]

Maps what differs between two commits onto the symbols of a Salesforce DX
project and writes changed.jsonl: the files that differ, the symbols the
difference adds, modifies and removes, and the names in the entries of
permission sets, profiles and custom metadata records that it touches.
It reads its two inputs and nothing else: no repository and no git.

  --delta <file>  delta.jsonl as sf-impact-delta writes it for the two commits;
                  `-` for standard input
  --refs <file>   the refs pack that sf-impact-extract writes for the tree of
                  the head commit; `-` for standard input
  --out <file>    where to write; standard output if absent or `-`
  --accept-unverified-refs
                  map onto refs of a tree that is no commit's: one with
                  changes that are not committed, or one in no repository.
                  Such refs cannot be known to be of the head and are
                  refused without this. The output says that it was given

The first row of each input says what it is of. Refs that are not of the
head of the delta, by its commit or its tree, its scope and its config, are
refused, with or without that option.
";

/// A file to read; `None` for standard input.
type Input = Option<PathBuf>;

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Changed {
        delta: Input,
        refs: Input,
        out: Option<PathBuf>,
        accept_unverified_refs: bool,
    },
    Help,
    Version,
}

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    sf_impact_common::allocator::return_freed_memory_at_once();
    let command = match parse_args(env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprint!("sf-impact-changed: {}\n\n{USAGE}", said(&message));
            return ExitCode::from(2);
        }
    };
    let (delta, refs, out, accept_unverified_refs) = match command {
        Command::Help => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("sf-impact-changed {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Changed {
            delta,
            refs,
            out,
            accept_unverified_refs,
        } => (delta, refs, out, accept_unverified_refs),
    };
    let lines = open(&delta)
        .and_then(|delta| sf_impact_changed::changed(delta, open(&refs)?, accept_unverified_refs));
    match lines.and_then(|lines| write(&lines, out.as_deref())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sf-impact-changed: {}", said(&message));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let (mut delta, mut refs, mut out) = (None, None, None);
    let mut accept_unverified_refs = false;
    let or_dash = |path: String| Some(path).filter(|path| path != "-").map(PathBuf::from);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--delta" => delta = Some(or_dash(value()?)),
            "--refs" => refs = Some(or_dash(value()?)),
            "--out" => out = or_dash(value()?),
            "--accept-unverified-refs" => accept_unverified_refs = true,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    let (delta, refs) = (
        delta.ok_or("--delta is required")?,
        refs.ok_or("--refs is required")?,
    );
    if delta.is_none() && refs.is_none() {
        return Err("only one of --delta and --refs can be standard input".into());
    }
    Ok(Command::Changed {
        delta,
        refs,
        out,
        accept_unverified_refs,
    })
}

fn open(input: &Input) -> Result<Box<dyn BufRead>, String> {
    match input {
        Some(path) => match fs::File::open(path) {
            Ok(file) => Ok(Box::new(BufReader::new(file))),
            Err(e) => Err(format!("{}: {e}", path.display())),
        },
        None => Ok(Box::new(io::stdin().lock())),
    }
}

fn write(lines: &[String], out: Option<&Path>) -> Result<(), String> {
    let write_to = |sink: &mut dyn Write| {
        let mut sink = io::BufWriter::new(sink);
        for line in lines {
            sink.write_all(line.as_bytes())?;
            sink.write_all(b"\n")?;
        }
        sink.flush()
    };
    match out {
        Some(path) => Replacement::new(path)
            .and_then(|mut file| {
                write_to(&mut file)?;
                file.finish(false)
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
    fn the_delta_the_refs_and_the_output_are_read_from_the_command_line() {
        let command = parse(&[
            "--delta",
            "delta.jsonl",
            "--refs",
            "refs.jsonl",
            "--out",
            "changed.jsonl",
        ]);

        assert_eq!(
            command,
            Ok(Command::Changed {
                delta: Some("delta.jsonl".into()),
                refs: Some("refs.jsonl".into()),
                out: Some("changed.jsonl".into()),
                accept_unverified_refs: false,
            })
        );
    }

    #[test]
    fn refs_that_cannot_be_verified_are_accepted_only_when_the_command_line_says_so() {
        let command = parse(&["--delta", "d", "--refs", "r", "--accept-unverified-refs"]);

        let Ok(Command::Changed {
            accept_unverified_refs,
            ..
        }) = command
        else {
            panic!("should parse");
        };
        assert!(accept_unverified_refs);
    }

    #[test]
    fn a_dash_is_standard_input_or_standard_output() {
        let command = parse(&["--delta", "-", "--refs", "refs.jsonl", "--out", "-"]);

        assert_eq!(
            command,
            Ok(Command::Changed {
                delta: None,
                refs: Some("refs.jsonl".into()),
                out: None,
                accept_unverified_refs: false,
            })
        );
    }

    #[test]
    fn bad_command_lines_are_refused_with_a_reason() {
        assert_eq!(parse(&["--refs", "r"]), Err("--delta is required".into()));
        assert_eq!(parse(&["--delta", "d"]), Err("--refs is required".into()));
        assert_eq!(parse(&["--delta"]), Err("--delta needs a value".into()));
        assert_eq!(
            parse(&["--delta", "-", "--refs", "-"]),
            Err("only one of --delta and --refs can be standard input".into())
        );
        assert!(parse(&["--delta", "d", "--refs", "r", "--base", "main"]).is_err());
    }
}
