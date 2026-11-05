use compiler233_dotnet::{parse, parser, syntax_issues};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs, hint::black_box, io::Write, path::Path, process::ExitCode, time::Instant};

fn run() -> Result<bool, String> {
    let args: Vec<String> = env::args().collect();
    let usage = "usage: compiler233-dotnet compile <file.cs> <output.dll> | check|check-subset <file.cs> | bench|bench-subset|bench-compile <file.cs> <iterations> <warmup> | csc @<response.rsp> [options]";
    if args.len() < 3 {
        return Err(usage.into());
    }
    let source = fs::read_to_string(&args[2]).map_err(|e| format!("{}: {e}", args[2]))?;
    match args[1].as_str() {
        "compile" if args.len() == 4 => {
            let output = Path::new(&args[3]);
            if !output
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
            {
                return Err("output must have a .dll extension".into());
            }
            if let Ok(destination) = output.canonicalize()
                && destination
                    == Path::new(&args[2])
                        .canonicalize()
                        .map_err(|e| e.to_string())?
            {
                return Err("output cannot overwrite the input".into());
            }
            let name = output
                .file_stem()
                .and_then(|n| n.to_str())
                .ok_or("invalid output filename")?;
            let bytes = match compiler233_dotnet::compile::compile(&source, name) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("{}: {e}", args[2]);
                    return Ok(false);
                }
            };
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            temporary.write_all(&bytes).map_err(|e| e.to_string())?;
            temporary.persist(output).map_err(|e| e.error.to_string())?;
            Ok(true)
        }
        "check-subset" if args.len() == 3 => match compiler233_dotnet::subset::parse(&source) {
            Ok(_) => Ok(true),
            Err(issue) => {
                eprintln!("{}:byte {}: {}", args[2], issue.offset, issue.message);
                Ok(false)
            }
        },
        "check" if args.len() == 3 => {
            let mut parser = parser();
            let tree = parse(&mut parser, &source);
            for issue in syntax_issues(&tree) {
                eprintln!(
                    "{}:{}:{}: {}",
                    args[2], issue.line, issue.column, issue.kind
                );
            }
            Ok(!tree.root_node().has_error())
        }
        "bench" if args.len() == 5 => {
            let mut parser = parser();
            let iterations: usize = args[3]
                .parse()
                .map_err(|_| "iterations must be an integer")?;
            let warmup: usize = args[4].parse().map_err(|_| "warmup must be an integer")?;
            if iterations == 0 {
                return Err("iterations must be positive".into());
            }
            let probe = parse(&mut parser, &source);
            let has_error = probe.root_node().has_error();
            for _ in 0..warmup {
                black_box(parse(&mut parser, black_box(&source)));
            }
            let mut samples = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                let start = Instant::now();
                let tree = parse(&mut parser, black_box(&source));
                black_box(tree.root_node().has_error());
                black_box(&tree);
                samples.push(start.elapsed().as_nanos() as u64);
            }
            samples.sort_unstable();
            let percentile =
                |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
            println!(
                "{}",
                json!({"engine":"tree-sitter-c-sharp (Rust host)","source_sha256":format!("{:x}", Sha256::digest(source.as_bytes())),"bytes":source.len(),"iterations":iterations,"warmup":warmup,"has_parse_issues":has_error,"p50_ns":percentile(50),"p95_ns":percentile(95),"samples_ns":samples})
            );
            Ok(true)
        }
        "bench-subset" | "bench-compile" if args.len() == 5 => {
            let iterations: usize = args[3]
                .parse()
                .map_err(|_| "iterations must be an integer")?;
            let warmup: usize = args[4].parse().map_err(|_| "warmup must be an integer")?;
            if iterations == 0 {
                return Err("iterations must be positive".into());
            }
            let probe = compiler233_dotnet::subset::parse(&source)
                .map_err(|e| format!("unsupported input at byte {}: {}", e.offset, e.message))?;
            let token_count = probe.tokens.len();
            let node_count = probe.nodes.len();
            if args[1] == "bench-compile" {
                let probe = compiler233_dotnet::compile::compile(&source, "Benchmark")
                    .map_err(|e| format!("cannot compile input: {e}"))?;
                let output_bytes = probe.len();
                for _ in 0..warmup {
                    black_box(
                        compiler233_dotnet::compile::compile(black_box(&source), "Benchmark")
                            .expect("validated source"),
                    );
                }
                let mut samples = Vec::with_capacity(iterations);
                for _ in 0..iterations {
                    let start = Instant::now();
                    let assembly =
                        compiler233_dotnet::compile::compile(black_box(&source), "Benchmark")
                            .expect("validated source");
                    black_box(&assembly);
                    samples.push(start.elapsed().as_nanos() as u64);
                }
                samples.sort_unstable();
                let percentile = |percent: usize| {
                    samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)]
                };
                println!(
                    "{}",
                    json!({"engine":"native Rust C# subset compiler", "stage":"parse, bind, emit", "source_sha256":format!("{:x}", Sha256::digest(source.as_bytes())), "bytes":source.len(), "iterations":iterations, "warmup":warmup, "has_parse_issues":false, "output_bytes":output_bytes, "p50_ns":percentile(50), "p95_ns":percentile(95), "samples_ns":samples})
                );
                return Ok(true);
            }
            for _ in 0..warmup {
                black_box(
                    compiler233_dotnet::subset::parse(black_box(&source))
                        .expect("validated source"),
                );
            }
            let mut samples = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                let start = Instant::now();
                let tree = compiler233_dotnet::subset::parse(black_box(&source))
                    .expect("validated source");
                black_box(tree.root);
                black_box(&tree);
                samples.push(start.elapsed().as_nanos() as u64);
            }
            samples.sort_unstable();
            let percentile =
                |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
            println!(
                "{}",
                json!({"engine":"native Rust C# subset", "source_sha256":format!("{:x}", Sha256::digest(source.as_bytes())), "bytes":source.len(), "iterations":iterations, "warmup":warmup, "has_parse_issues":false, "token_count":token_count, "node_count":node_count, "p50_ns":percentile(50), "p95_ns":percentile(95), "samples_ns":samples})
            );
            Ok(true)
        }
        _ => Err(usage.into()),
    }
}

/// Accepts the spellings a csc consumer uses: an explicit `csc` subcommand, a
/// compiler-style invocation that starts with an option or response file, or a
/// bare C# source path.
fn csc_arguments(args: &[String]) -> Option<&[String]> {
    let first = args.first()?;
    if first.eq_ignore_ascii_case("csc") {
        return Some(&args[1..]);
    }
    let looks_like_source = Path::new(first)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cs"));
    if first.starts_with('-') || first.starts_with('@') || looks_like_source {
        return Some(args);
    }
    None
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "version" | "--version" | "-V") {
        println!("compiler233-dotnet {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|first| first == "frontend") {
        return ExitCode::from(compiler233_dotnet::frontend::run(&args[1..]));
    }
    if let Some(csc_args) = csc_arguments(&args) {
        return ExitCode::from(compiler233_dotnet::csc::run(csc_args));
    }
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
