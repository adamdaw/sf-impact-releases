//! `sf-impact-delta`: compares two commits of the git repository that holds a
//! Salesforce DX project and writes `delta.jsonl`: the files that differ,
//! their lines that differ and the text of both versions. The rows are
//! defined by `schema/delta.schema.json`.

use sf_impact_common::diagnostic::said;
use sf_impact_common::files::Replacement;
use sf_impact_delta::Base;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: sf-impact-delta (--base <rev> | --merge-base <rev>) --head <rev>
                 [--root <dir>] [--trust-repo] [--out <file>]

Compares two commits of the git repository that holds a Salesforce DX
project and writes delta.jsonl: the two commits, every file that is not the
same in both, the lines of it that differ, and the text of both versions.

  --base <rev>        the commit to compare from, used as it is given. For a
                      branch that is to be merged, give the merge base, not
                      the tip of the branch it goes into
  --merge-base <rev>  compare from the merge base of <rev> and the head
  --head <rev>        the commit to compare to
  --root <dir>        the project: the directory with sfdx-project.json, in
                      the repository; the current directory if absent
  --trust-repo        read the repository though another user owns it, as
                      in a container whose workspace someone else made. Git
                      refuses to on its own. Only this repository is trusted,
                      and nothing is written to any configuration
  --out <file>        where to write; standard output if absent or `-`

Git reads the objects the repository has and fetches none: in a partial
clone, fetch the files of the two commits first.
";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Delta {
        base: Base,
        head: String,
        root: PathBuf,
        trust_repo: bool,
        out: Option<PathBuf>,
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
            eprint!("sf-impact-delta: {}\n\n{USAGE}", said(&message));
            return ExitCode::from(2);
        }
    };
    let (base, head, root, trust_repo, out) = match command {
        Command::Help => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("sf-impact-delta {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Delta {
            base,
            head,
            root,
            trust_repo,
            out,
        } => (base, head, root, trust_repo, out),
    };
    let lines = sf_impact_delta::delta(&root, trust_repo, &base, &head);
    match lines.and_then(|lines| write(&lines, out.as_deref())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sf-impact-delta: {}", said(&message));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let (mut base, mut head, mut root, mut out) = (None, None, None, None);
    let mut trust_repo = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--base" | "--merge-base" if base.is_some() => {
                return Err("give one of --base and --merge-base, once".into());
            }
            "--base" => base = Some(Base::Commit(value()?)),
            "--merge-base" => base = Some(Base::MergeBaseWith(value()?)),
            "--head" => head = Some(value()?),
            "--root" => root = Some(PathBuf::from(value()?)),
            "--trust-repo" => trust_repo = true,
            "--out" => out = Some(value()?).filter(|out| out != "-").map(PathBuf::from),
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    Ok(Command::Delta {
        base: base.ok_or("--base or --merge-base is required")?,
        head: head.ok_or("--head is required")?,
        root: root.unwrap_or_else(|| PathBuf::from(".")),
        trust_repo,
        out,
    })
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
    fn the_commits_the_root_the_trust_and_the_output_are_read_from_the_command_line() {
        let command = parse(&[
            "--base",
            "main",
            "--head",
            "HEAD",
            "--root",
            "repo",
            "--trust-repo",
            "--out",
            "delta.jsonl",
        ]);

        assert_eq!(
            command,
            Ok(Command::Delta {
                base: Base::Commit("main".into()),
                head: "HEAD".into(),
                root: "repo".into(),
                trust_repo: true,
                out: Some("delta.jsonl".into()),
            })
        );
    }

    #[test]
    fn by_default_the_root_is_the_current_directory_the_output_standard_output_and_no_trust_given()
    {
        let command = parse(&["--merge-base", "main", "--head", "HEAD", "--out", "-"]);

        assert_eq!(
            command,
            Ok(Command::Delta {
                base: Base::MergeBaseWith("main".into()),
                head: "HEAD".into(),
                root: ".".into(),
                trust_repo: false,
                out: None,
            })
        );
    }

    #[test]
    fn bad_command_lines_are_refused_with_a_reason() {
        assert_eq!(
            parse(&["--head", "h"]),
            Err("--base or --merge-base is required".into())
        );
        assert_eq!(parse(&["--base", "main"]), Err("--head is required".into()));
        assert_eq!(
            parse(&["--head", "h", "--base"]),
            Err("--base needs a value".into())
        );
        assert!(parse(&["--head", "h", "--base", "a", "--merge-base", "b"]).is_err());
        assert!(parse(&["--head", "h", "--base", "a", "--frobnicate"]).is_err());
    }
}
