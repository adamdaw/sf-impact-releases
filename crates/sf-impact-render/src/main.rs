use sf_impact_common::diagnostic::said;
use sf_impact_common::files::Replacement;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
    process::ExitCode,
};
const USAGE: &str = "usage: sf-impact-render --impact <impact.json> --format md|gh-step [--out <file>|stdout]\n\nReads impact.json only; - reads stdin. Output defaults to stdout.\nmd has a 60000-byte budget, groups confidence reasons and counts omitted\nrecords; retain impact.json as an artifact. gh-step emits selection=empty\nwhen no tests are selected: check it before using RunSpecifiedTests.\n";
struct Args {
    impact: String,
    format: String,
    out: Option<String>,
}
fn parse() -> Result<Args, String> {
    let mut args = env::args().skip(1);
    let (mut impact, mut format, mut out) = (None, None, None);
    while let Some(key) = args.next() {
        let slot = match key.as_str() {
            "--impact" => &mut impact,
            "--format" => &mut format,
            "--out" => &mut out,
            _ => return Err(format!("unexpected argument {key:?}")),
        };
        if slot.is_some() {
            return Err(format!("duplicate {key}"));
        }
        *slot = Some(
            args.next()
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| format!("{key} needs a value"))?,
        );
    }
    let format = format.ok_or("--format is required")?;
    if !["md", "gh-step"].contains(&format.as_str()) {
        return Err("--format must be md or gh-step".into());
    }
    Ok(Args {
        impact: impact.ok_or("--impact is required")?,
        format,
        out,
    })
}
fn run(args: Args) -> Result<(), String> {
    let bytes = if args.impact == "-" {
        let mut bytes = Vec::new();
        io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        bytes
    } else {
        fs::read(&args.impact).map_err(|e| format!("{}: {e}", args.impact))?
    };
    // Validate and render before opening the destination: a refusal leaves it intact.
    let rendered = sf_impact_render::render(&bytes, &args.format)?;
    match args.out.as_deref() {
        None | Some("-" | "stdout") => match io::stdout().lock().write_all(rendered.as_bytes()) {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            result => result.map_err(|e| e.to_string()),
        },
        Some(path) => Replacement::new(Path::new(path))
            .and_then(|mut file| {
                file.write_all(rendered.as_bytes())?;
                file.finish(false)
            })
            .map_err(|e| format!("{path}: {e}")),
    }
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
        println!("sf-impact-render {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let (result, code) = match parse() {
        Ok(args) => (run(args), 1),
        Err(e) => (Err(e), 2),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sf-impact-render: {}", said(&e.replace(['\r', '\n'], " ")));
            ExitCode::from(code)
        }
    }
}
