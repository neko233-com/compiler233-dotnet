use compiler233_dotnet::{parse, parser, syntax_issues};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs, hint::black_box, process::ExitCode, time::Instant};

fn run() -> Result<bool, String> {
    let args: Vec<String> = env::args().collect();
    let usage = "usage: compiler233-dotnet check <file.cs> | bench <file.cs> <iterations> <warmup>";
    if args.len() < 3 {
        return Err(usage.into());
    }
    let source = fs::read_to_string(&args[2]).map_err(|e| format!("{}: {e}", args[2]))?;
    let mut parser = parser();
    match args[1].as_str() {
        "check" if args.len() == 3 => {
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
        _ => Err(usage.into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
