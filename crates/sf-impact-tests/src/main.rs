use sf_impact_common::diagnostic::{said, shown};
use sf_impact_common::files::{self, Replacement};
use sf_impact_tests::{Config, write_impact_kept};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::ExitCode,
};
const USAGE: &str = "usage: sf-impact-tests --refs <file> (--changed <file> | --symbol <name>...) [--depth N] [--config <file>] [--out <file>|stdout] [--accept-unverified-refs]\n\nReads only refs, changed and optional config. Refs are the refs pack that\nsf-impact-extract writes. Default depth: 2.\nUse - for one input on stdin; output defaults to stdout. Unknown symbols\nproduce error rows and exit 1; invalid arguments exit 2.\n\nThe first row of each input says what it is of. With --changed, refs that\nare not of the head it was made for, by its commit or its tree, its scope\nand its config, are refused and exit 1. Refs of a tree that is no commit's\ncannot be known to be: they are refused too, unless\n--accept-unverified-refs is given, and then the document says so in its\nconfidence reasons.\n";
struct Args {
    refs: String,
    changed: Option<String>,
    symbols: Vec<String>,
    depth: usize,
    config: Option<String>,
    out: Option<String>,
    accept_unverified_refs: bool,
}
fn parse(args: Vec<String>) -> Result<Args, String> {
    let mut iter = args.into_iter().peekable();
    let (mut refs, mut changed, mut config, mut out) = (None, None, None, None);
    let mut symbols = Vec::new();
    let mut depth = 2;
    let mut accept_unverified_refs = false;
    while let Some(arg) = iter.next() {
        if arg == "--accept-unverified-refs" {
            accept_unverified_refs = true;
            continue;
        }
        if arg == "--symbol" {
            let start = symbols.len();
            while iter.peek().is_some_and(|s| !s.starts_with('-')) {
                symbols.push(sf_impact_common::types::normal_symbol(
                    &iter.next().unwrap(),
                ));
            }
            if symbols.len() == start {
                return Err("--symbol needs a name".into());
            }
            continue;
        }
        let mut value = || {
            iter.next()
                .filter(|s| !s.starts_with("--"))
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--refs" => refs = Some(value()?),
            "--changed" => changed = Some(value()?),
            "--config" => config = Some(value()?),
            "--out" => out = Some(value()?),
            "--depth" => {
                depth = value()?
                    .parse()
                    .map_err(|_| "--depth must be a nonnegative integer")?
            }
            _ => return Err(format!("unexpected argument `{arg}`")),
        }
    }
    let refs = refs.ok_or("--refs is required")?;
    if changed.is_some() == !symbols.is_empty() {
        return Err("give exactly one of --changed or --symbol".into());
    }
    if refs == "-" && changed.as_deref() == Some("-") {
        return Err("only one input can use stdin".into());
    }
    Ok(Args {
        refs,
        changed,
        symbols,
        depth,
        config,
        out,
        accept_unverified_refs,
    })
}
fn open(path: &str) -> Result<Box<dyn BufRead>, String> {
    if path == "-" {
        Ok(Box::new(io::stdin().lock()))
    } else {
        fs::File::open(path)
            .map(|f| Box::new(BufReader::new(f)) as Box<dyn BufRead>)
            .map_err(|e| format!("{path}: {e}"))
    }
}
fn run(args: Args) -> Result<bool, String> {
    let config = match args.config {
        Some(path) => {
            let bytes = files::read_regular(Path::new(&path), files::MAX_CONFIG_BYTES);
            serde_json::from_slice::<Config>(&bytes.map_err(|e| format!("{path}: {e}"))?)
                .map_err(|e| format!("{path}: {}", shown(&e.to_string())))?
        }
        None => Config::default(),
    };
    let changed = match args.changed {
        Some(path) => {
            let mut bytes = Vec::new();
            open(&path)?
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{path}: {e}"))?;
            Some(bytes)
        }
        None => None,
    };
    let mut file = match args.out.as_deref() {
        None | Some("-" | "stdout") => None,
        Some(path) => Some(Replacement::new(Path::new(path)).map_err(|e| format!("{path}: {e}"))?),
    };
    let mut sink: Box<dyn Write + '_> = match &mut file {
        None => Box::new(io::BufWriter::new(io::stdout().lock())),
        Some(file) => Box::new(io::BufWriter::new(file)),
    };
    let result = write_impact_kept(
        open(&args.refs)?,
        changed.as_deref(),
        &args.symbols,
        args.depth,
        &config,
        args.accept_unverified_refs,
        &mut sink,
    );
    let flushed = sink.flush();
    drop(sink);
    // Deliberate: the process is about to exit, and freeing the graph and
    // the rows one allocation at a time is a measurable share of the run.
    // The OS reclaims it all at once. Only after the flush, so nothing
    // written is lost; the library's write_impact frees as usual.
    let result = result.map(|(ok, kept)| {
        std::mem::forget(kept);
        ok
    });
    match flushed {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(true),
        Err(e) => return Err(e.to_string()),
        _ => {}
    }
    if let (Some(file), Some(path)) = (file, &args.out) {
        file.finish(false).map_err(|e| format!("{path}: {e}"))?;
    }
    result
}
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    sf_impact_common::allocator::return_freed_memory_at_once();
    let args: Vec<_> = env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args == ["--version"] || args == ["-V"] {
        println!("sf-impact-tests {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let args = match parse(args) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("sf-impact-tests: {}\n\n{USAGE}", said(&e));
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("sf-impact-tests: {}", said(&e));
            ExitCode::FAILURE
        }
    }
}
