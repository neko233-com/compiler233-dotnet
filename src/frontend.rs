//! Batch syntax verification: parse many files in one process, write nothing
//! except an optional JSON report. This is the fast loop the differential gate
//! uses, so a corpus run costs one process instead of one per file.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::json;

use crate::syntax;

const USAGE: &str = "usage: compiler233-dotnet frontend <file.cs|manifest.txt>... [-report:<path>] [-define:<symbol>]...";

pub fn run(args: &[String]) -> u8 {
    let started = Instant::now();
    let mut inputs: Vec<String> = Vec::new();
    let mut report: Option<PathBuf> = None;
    let mut defines: Vec<String> = Vec::new();
    for argument in args {
        if let Some(value) = argument
            .strip_prefix("-report:")
            .or_else(|| argument.strip_prefix("--report:"))
        {
            report = Some(PathBuf::from(value));
        } else if let Some(value) = argument
            .strip_prefix("-define:")
            .or_else(|| argument.strip_prefix("--define:"))
        {
            defines.extend(
                value
                    .split([';', ','])
                    .filter(|symbol| !symbol.is_empty())
                    .map(str::to_string),
            );
        } else if argument.starts_with('-') && argument != "-" {
            eprintln!("error CD2330101: unknown option '{argument}'");
            eprintln!("{USAGE}");
            return 2;
        } else {
            inputs.push(argument.clone());
        }
    }
    if inputs.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }

    let files = match collect_files(&inputs) {
        Ok(files) => files,
        Err(message) => {
            eprintln!("error CD2330102: {message}");
            return 2;
        }
    };
    if files.is_empty() {
        eprintln!("error CD2330103: no C# files were found");
        return 2;
    }

    let mut entries = Vec::with_capacity(files.len());
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut types = 0usize;
    let mut members = 0usize;
    for file in &files {
        let display = file.display().to_string();
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                rejected += 1;
                entries.push(json!({
                    "path": display,
                    "ok": false,
                    "error": { "line": null, "column": null, "message": format!("not readable UTF-8 text: {error}") },
                }));
                continue;
            }
        };
        match syntax::parse_with_defines(&source, &defines) {
            Ok(unit) => {
                accepted += 1;
                types += unit.types;
                members += unit.members;
                entries.push(json!({
                    "path": display,
                    "ok": true,
                    "types": unit.types,
                    "members": unit.members,
                }));
            }
            Err(error) => {
                rejected += 1;
                let (line, column) = error.line_column(&source);
                entries.push(json!({
                    "path": display,
                    "ok": false,
                    "error": { "line": line, "column": column, "message": error.message },
                }));
            }
        }
    }

    let document = json!({
        "tool": "compiler233-dotnet",
        "mode": "frontend",
        "milestone": "M2",
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "define": defines,
        "file_count": files.len(),
        "accepted": accepted,
        "rejected": rejected,
        "types": types,
        "members": members,
        "files": entries,
    });
    let serialized = serde_json::to_vec_pretty(&document).expect("JSON serialization cannot fail");

    match report {
        Some(path) => {
            if let Err(error) = write_atomic(&path, &serialized) {
                eprintln!(
                    "error CD2330104: cannot write report to {}: {error}",
                    path.display()
                );
                return 2;
            }
            println!(
                "frontend: {} files | accepted {accepted} | rejected {rejected} ({:?})",
                files.len(),
                started.elapsed()
            );
        }
        None => {
            println!("{}", String::from_utf8_lossy(&serialized));
        }
    }

    if rejected == 0 { 0 } else { 1 }
}

fn collect_files(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for input in inputs {
        let path = PathBuf::from(input);
        if !path.exists() {
            return Err(format!("input not found: {input}"));
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read manifest {input}: {error}"))?;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                push_unique(&mut files, &mut seen, PathBuf::from(line));
            }
        } else {
            push_unique(&mut files, &mut seen, path);
        }
    }
    Ok(files)
}

fn push_unique(
    files: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<String>,
    path: PathBuf,
) {
    let key = path.to_string_lossy().to_lowercase();
    if seen.insert(key) {
        files.push(path);
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
