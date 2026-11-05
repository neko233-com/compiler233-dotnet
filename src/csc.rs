//! Roslyn `csc`-compatible command line front end (milestone M0).
//!
//! The contract implemented here is the one Unity, Tuanjie and HotReload233
//! actually consume: response files (`@file`), the compiler switches they pass,
//! csc-shaped diagnostics with matching exit codes, and an assembly or reference
//! assembly on the requested paths. Language coverage is still the restricted
//! subset in [`crate::compile`]; anything unsupported fails closed with an
//! explicit diagnostic instead of emitting a wrong assembly.
use std::{
    collections::BTreeSet,
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::{Value, json};

use crate::{compile, subset};

const MAX_RESPONSE_DEPTH: usize = 16;
const MAX_FRONTEND_EXAMPLES: usize = 20;
const FRONTEND_ENGINE: &str = "tree-sitter-c-sharp";

const USAGE: &str = concat!(
    "compiler233-dotnet csc -- Roslyn csc compatible front end (M0)\n",
    "  compiler233-dotnet @response.rsp\n",
    "  compiler233-dotnet -out:<path> -target:library -reference:<path> ... <file.cs>\n",
    "  -report:<path>   write a machine readable JSON report of this invocation\n",
    "  -help            show this help\n",
    "Unsupported constructs report an error and produce no assembly.\n",
);

/// Switches that are accepted but intentionally not applied. They do not change
/// the meaning of the assembly this compiler currently produces; the report
/// records them so a caller can see what was dropped.
const IGNORED_OPTIONS: &[&str] = &[
    "additionalfile",
    "analyzer",
    "analyzerconfig",
    "autoref",
    "checked",
    "debug",
    "debugtype",
    "define",
    "deterministic",
    "doc",
    "embed",
    "errorendlocation",
    "errorlog",
    "features",
    "filealign",
    "fullpaths",
    "highentropyva",
    "instrument",
    "lib",
    "main",
    "moduleassemblyname",
    "noconfig",
    "nologo",
    "nowarn",
    "optimize",
    "parallel",
    "pathmap",
    "platform",
    "preferreduilang",
    "refonly",
    "ruleset",
    "simplifiedpaths",
    "sourcelink",
    "subsystemversion",
    "unsafe",
    "utf8output",
    "warn",
    "warnaserror",
    "warnnotaserror",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

impl Diagnostic {
    fn error(file: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            file: file.into(),
            line: None,
            column: None,
            severity: Severity::Error,
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn at(
        file: impl Into<String>,
        line: usize,
        column: usize,
        severity: Severity,
        code: &str,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            file: file.into(),
            line: Some(line),
            column: Some(column),
            severity,
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn warning(file: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            file: file.into(),
            line: None,
            column: None,
            severity: Severity::Warning,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(formatter, "{}({line},{column}): ", self.file)?,
            _ => write!(formatter, "{}: ", self.file)?,
        }
        write!(
            formatter,
            "{} {}: {}",
            self.severity.as_str(),
            self.code,
            self.message
        )
    }
}

#[derive(Debug, Default)]
pub struct Options {
    target: Option<String>,
    output: Option<String>,
    refout: Option<String>,
    defines: BTreeSet<String>,
    references: Vec<String>,
    langversion: Option<String>,
    report: Option<String>,
    sources: Vec<String>,
    ignored: BTreeSet<String>,
}

impl Options {
    fn target(&self) -> &str {
        self.target.as_deref().unwrap_or("library")
    }

    /// csc derives the module name from the first source file when `-out` is
    /// absent and writes the assembly into the current working directory.
    fn output_path(&self) -> PathBuf {
        if let Some(output) = &self.output {
            return PathBuf::from(output);
        }
        let first = self
            .sources
            .first()
            .and_then(|source| Path::new(source).file_stem())
            .and_then(|stem| stem.to_str())
            .unwrap_or("output");
        PathBuf::from(format!("{first}.dll"))
    }
}

/// Splits one command line or response-file line into arguments. Quotes group
/// text and are removed; `""` inside a quoted run yields a literal quote, which
/// matches how Roslyn parses Unity's generated response files.
pub fn split_arguments(text: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if quoted {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    current.push('"');
                } else {
                    quoted = false;
                }
            } else {
                current.push(character);
            }
            started = true;
        } else if character == '"' {
            quoted = true;
            started = true;
        } else if character.is_whitespace() {
            if started {
                arguments.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(character);
            started = true;
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

/// Recognizes `-name:value` and, on Windows only, the `/name:value` spelling.
/// A leading slash is a path on Unix-like hosts, so it is never an option there.
fn split_option(token: &str) -> Option<(String, String)> {
    let rest = if let Some(rest) = token.strip_prefix('-') {
        rest
    } else if cfg!(windows) && token.len() > 1 && token.starts_with('/') {
        &token[1..]
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }
    let (name, value) = match rest.find(':') {
        Some(index) => (&rest[..index], &rest[index + 1..]),
        None => (rest, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_ascii_lowercase(), value.to_string()))
}

fn expand_arguments(args: &[String], diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut stack = Vec::new();
    for argument in args {
        expand_argument(argument, 0, &mut stack, &mut expanded, diagnostics);
    }
    expanded
}

fn expand_argument(
    argument: &str,
    depth: usize,
    stack: &mut Vec<PathBuf>,
    expanded: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(rest) = argument.strip_prefix('@') else {
        expanded.push(argument.to_string());
        return;
    };
    if rest.is_empty() {
        expanded.push(argument.to_string());
        return;
    }
    if depth >= MAX_RESPONSE_DEPTH {
        diagnostics.push(Diagnostic::error(
            rest,
            "CD2330006",
            format!("response files are nested deeper than {MAX_RESPONSE_DEPTH} levels"),
        ));
        return;
    }
    let path = PathBuf::from(rest);
    let Ok(canonical) = path.canonicalize() else {
        diagnostics.push(Diagnostic::error(
            rest,
            "CD2330007",
            format!("response file '{rest}' was not found"),
        ));
        return;
    };
    if stack.contains(&canonical) {
        diagnostics.push(Diagnostic::error(
            rest,
            "CD2330008",
            format!("response file '{rest}' includes itself"),
        ));
        return;
    }
    let Ok(text) = fs::read_to_string(&canonical) else {
        diagnostics.push(Diagnostic::error(
            rest,
            "CD2330009",
            format!("response file '{rest}' is not readable UTF-8 text"),
        ));
        return;
    };
    stack.push(canonical);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}');
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        for argument in split_arguments(line) {
            expand_argument(&argument, depth + 1, stack, expanded, diagnostics);
        }
    }
    stack.pop();
}

fn parse_options(tokens: &[String], options: &mut Options, diagnostics: &mut Vec<Diagnostic>) {
    for token in tokens {
        let Some((name, value)) = split_option(token) else {
            options.sources.push(token.clone());
            continue;
        };
        match name.as_str() {
            "target" | "t" => require_value(token, &value, diagnostics, |value| {
                options.target = Some(value.to_ascii_lowercase());
            }),
            "out" => require_value(token, &value, diagnostics, |value| {
                options.output = Some(value);
            }),
            "refout" => require_value(token, &value, diagnostics, |value| {
                options.refout = Some(value);
            }),
            "report" => require_value(token, &value, diagnostics, |value| {
                options.report = Some(value);
            }),
            "define" | "d" => {
                for define in value.split([';', ',']) {
                    if !define.is_empty() {
                        options.defines.insert(define.to_string());
                    }
                }
            }
            "reference" | "r" => {
                if !value.is_empty() {
                    options.references.push(value);
                }
            }
            "langversion" => {
                if value.is_empty() {
                    unknown_option(token, diagnostics);
                } else if is_known_langversion(&value) {
                    options.langversion = Some(value);
                } else {
                    diagnostics.push(Diagnostic::error(
                        "<command line>",
                        "CS1617",
                        format!("Invalid option '{value}' for /langversion"),
                    ));
                }
            }
            "nostdlib" | "nostdlib+" => diagnostics.push(Diagnostic::error(
                "<command line>",
                "CD2330002",
                "-nostdlib is not implemented; no assembly was produced",
            )),
            "resource" | "linkresource" | "win32res" | "win32icon" | "win32manifest"
            | "applicationicon" | "keyfile" | "delaysign" | "publicsign" => {
                diagnostics.push(Diagnostic::error(
                    "<command line>",
                    "CD2330002",
                    format!("{token} is not implemented; no assembly was produced"),
                ))
            }
            other => {
                let base = other
                    .strip_suffix('+')
                    .or_else(|| other.strip_suffix('-'))
                    .unwrap_or(other);
                if IGNORED_OPTIONS.contains(&base) {
                    options.ignored.insert(base.to_string());
                } else {
                    unknown_option(token, diagnostics);
                }
            }
        }
    }
}

fn require_value(
    token: &str,
    value: &str,
    diagnostics: &mut Vec<Diagnostic>,
    apply: impl FnOnce(String),
) {
    if value.is_empty() {
        unknown_option(token, diagnostics);
    } else {
        apply(value.to_string());
    }
}

fn unknown_option(token: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic::error(
        "<command line>",
        "CS2007",
        format!("Unrecognized option: '{token}'"),
    ));
}

fn is_known_langversion(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "default" | "latest" | "latestmajor" | "preview" | "iso-1" | "iso-2"
    ) {
        return true;
    }
    match lowered.split_once('.') {
        Some((major, minor)) => match (major.parse::<u32>(), minor.parse::<u32>()) {
            (Ok(7), Ok(minor)) => minor <= 3,
            (Ok(major), Ok(0)) => (1..=13).contains(&major),
            _ => false,
        },
        None => matches!(
            lowered.as_str(),
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11" | "12" | "13"
        ),
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let end = offset.min(source.len());
    let prefix = &source[..end];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = match prefix.rfind('\n') {
        Some(index) => prefix.len() - index,
        None => prefix.len() + 1,
    };
    (line, column)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn compile_assembly(
    options: &Options,
    diagnostics: &mut Vec<Diagnostic>,
    frontend_scan: &mut Option<Value>,
) -> Option<Vec<u8>> {
    let target = options.target();
    if !target.eq_ignore_ascii_case("library") {
        diagnostics.push(Diagnostic::error(
            "<command line>",
            "CD2330002",
            format!(
                "target '{target}' is not implemented; only library assemblies can be produced"
            ),
        ));
        return None;
    }
    if options.sources.is_empty() {
        diagnostics.push(Diagnostic::error(
            "<command line>",
            "CS2008",
            "No inputs specified",
        ));
        return None;
    }
    for source in &options.sources {
        if source == "-" {
            diagnostics.push(Diagnostic::error(
                "-",
                "CD2330004",
                "reading source from stdin is not implemented",
            ));
        } else if !Path::new(source).is_file() {
            diagnostics.push(Diagnostic::error(
                source.clone(),
                "CS2001",
                "Source file could not be found",
            ));
        }
    }
    if diagnostics.iter().any(Diagnostic::is_error) {
        return None;
    }

    if options.sources.len() > 1 {
        *frontend_scan = Some(scan_frontend(&options.sources));
        diagnostics.push(Diagnostic::error(
            options.sources[0].clone(),
            "CD2330003",
            format!(
                "multi-file compilation is not implemented ({} source files); no assembly was produced",
                options.sources.len()
            ),
        ));
        return None;
    }

    let source_path = &options.sources[0];
    let source = match fs::read_to_string(source_path) {
        Ok(source) => source,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                source_path.clone(),
                "CD2330004",
                format!("source file is not readable UTF-8 text: {error}"),
            ));
            return None;
        }
    };

    if let Err(issue) = subset::parse(&source) {
        let (line, column) = line_column(&source, issue.offset);
        diagnostics.push(Diagnostic::at(
            source_path.clone(),
            line,
            column,
            Severity::Error,
            "CD2330010",
            issue.message,
        ));
        return None;
    }

    let output = options.output_path();
    let Some(assembly_name) = output.file_stem().and_then(|stem| stem.to_str()) else {
        diagnostics.push(Diagnostic::error(
            "<command line>",
            "CD2330005",
            format!("output path '{}' has no usable file name", output.display()),
        ));
        return None;
    };
    let bytes = match compile::compile(&source, assembly_name) {
        Ok(bytes) => bytes,
        Err(message) => {
            diagnostics.push(Diagnostic::error(source_path.clone(), "CD2330011", message));
            return None;
        }
    };
    if let Err(error) = write_atomic(&output, &bytes) {
        diagnostics.push(Diagnostic::error(
            output.display().to_string(),
            "CS2012",
            format!("Cannot open '{}' for writing: {error}", output.display()),
        ));
        return None;
    }
    if let Some(refout) = &options.refout {
        let refout_path = PathBuf::from(refout);
        // Reference assemblies are a compile-time artifact for downstream
        // callers; M0 writes the full assembly instead of stripping method
        // bodies, so the deviation is reported rather than hidden.
        diagnostics.push(Diagnostic::warning(
            refout.clone(),
            "CD2330020",
            "-refout is not implemented; the full assembly was written instead",
        ));
        if let Err(error) = write_atomic(&refout_path, &bytes) {
            diagnostics.push(Diagnostic::error(
                refout.clone(),
                "CS2012",
                format!("Cannot open '{refout}' for writing: {error}"),
            ));
            return None;
        }
    }
    Some(bytes)
}

/// Measured, not claimed: how the current syntax front end fares on every
/// source file of a request it cannot compile yet.
fn scan_frontend(sources: &[String]) -> Value {
    let mut parser = crate::parser();
    let mut files = 0usize;
    let mut unreadable = 0usize;
    let mut files_with_issues = 0usize;
    let mut issue_count = 0usize;
    let mut examples = Vec::new();
    for source in sources {
        let Ok(text) = fs::read_to_string(source) else {
            unreadable += 1;
            continue;
        };
        files += 1;
        let tree = crate::parse(&mut parser, &text);
        let issues = crate::syntax_issues(&tree);
        if issues.is_empty() {
            continue;
        }
        files_with_issues += 1;
        issue_count += issues.len();
        if examples.len() < MAX_FRONTEND_EXAMPLES {
            let first = &issues[0];
            examples.push(json!({
                "file": source,
                "issue_count": issues.len(),
                "first": {
                    "line": first.line,
                    "column": first.column,
                    "kind": first.kind,
                },
            }));
        }
    }
    json!({
        "engine": FRONTEND_ENGINE,
        "files": files,
        "unreadable_files": unreadable,
        "files_with_syntax_issues": files_with_issues,
        "issue_count": issue_count,
        "examples": examples,
    })
}

fn write_report(
    path: &str,
    argv: &[String],
    options: &Options,
    diagnostics: &[Diagnostic],
    output_bytes: usize,
    frontend_scan: Option<Value>,
    elapsed_millis: u64,
) -> std::io::Result<()> {
    let failed = diagnostics.iter().any(Diagnostic::is_error);
    let report = json!({
        "tool": "compiler233-dotnet",
        "mode": "csc",
        "milestone": "M0",
        "elapsed_ms": elapsed_millis,
        "argument_count": argv.len(),
        "source_count": options.sources.len(),
        "sources": options.sources,
        "reference_count": options.references.len(),
        "target": options.target(),
        "langversion": options.langversion,
        "output": options.output,
        "refout": options.refout,
        "define": options.defines.iter().collect::<Vec<_>>(),
        "ignored_options": options.ignored.iter().collect::<Vec<_>>(),
        "outcome": if failed { "error" } else { "success" },
        "output_bytes": output_bytes,
        "diagnostics": diagnostics
            .iter()
            .map(|diagnostic| json!({
                "severity": diagnostic.severity.as_str(),
                "code": diagnostic.code,
                "file": diagnostic.file,
                "line": diagnostic.line,
                "column": diagnostic.column,
                "message": diagnostic.message,
            }))
            .collect::<Vec<_>>(),
        "frontend_scan": frontend_scan,
    });
    let bytes = serde_json::to_vec_pretty(&report).map_err(std::io::Error::other)?;
    write_atomic(Path::new(path), &bytes)
}

/// Runs one `csc` invocation and returns the process exit code: 0 on success,
/// 1 when any error was reported, matching Roslyn's command line compiler.
pub fn run(args: &[String]) -> u8 {
    let started = Instant::now();
    if args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "-?" | "-help" | "--help" | "/?" | "/help"
        )
    }) {
        println!("{USAGE}");
        return 0;
    }

    let mut diagnostics = Vec::new();
    let expanded = expand_arguments(args, &mut diagnostics);
    let mut options = Options::default();
    if diagnostics.is_empty() {
        parse_options(&expanded, &mut options, &mut diagnostics);
    }

    let mut output_bytes = 0;
    let mut frontend_scan = None;
    if !diagnostics.iter().any(Diagnostic::is_error)
        && let Some(bytes) = compile_assembly(&options, &mut diagnostics, &mut frontend_scan)
    {
        output_bytes = bytes.len();
    }

    for diagnostic in &diagnostics {
        println!("{diagnostic}");
    }

    if let Some(report) = &options.report
        && let Err(error) = write_report(
            report,
            args,
            &options,
            &diagnostics,
            output_bytes,
            frontend_scan,
            started.elapsed().as_millis() as u64,
        )
    {
        println!(
            "{}",
            Diagnostic::warning(
                report.clone(),
                "CD2330021",
                format!("cannot write the report: {error}")
            )
        );
    }

    if diagnostics.iter().any(Diagnostic::is_error) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(line: &str) -> Vec<String> {
        split_arguments(line)
    }

    #[test]
    fn splits_quoted_arguments() {
        assert_eq!(
            tokens(r#"-out:"C:/a b/patch.dll" -define:A;B "C:/x y/A.cs""#),
            vec![
                "-out:C:/a b/patch.dll".to_string(),
                "-define:A;B".to_string(),
                "C:/x y/A.cs".to_string()
            ]
        );
        assert_eq!(
            tokens(r#""-out:C:/q.dll""#),
            vec!["-out:C:/q.dll".to_string()]
        );
        assert_eq!(tokens(r#""a""b""#), vec!["a\"b".to_string()]);
    }

    #[test]
    fn parses_core_options() {
        let mut options = Options::default();
        let mut diagnostics = Vec::new();
        parse_options(
            &[
                "-target:library".to_string(),
                "-out:patch.dll".to_string(),
                "-refout:patch.ref.dll".to_string(),
                "-define:HOTRELOAD233_PATCH;UNITY_2021".to_string(),
                "-r:C:/refs/a.dll".to_string(),
                "-langversion:9.0".to_string(),
                "-nologo".to_string(),
                "-optimize+".to_string(),
                "A.cs".to_string(),
                "B.cs".to_string(),
            ],
            &mut options,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(options.target(), "library");
        assert_eq!(options.output.as_deref(), Some("patch.dll"));
        assert_eq!(options.refout.as_deref(), Some("patch.ref.dll"));
        assert_eq!(options.references.len(), 1);
        assert_eq!(options.langversion.as_deref(), Some("9.0"));
        assert!(options.defines.contains("HOTRELOAD233_PATCH"));
        assert!(options.defines.contains("UNITY_2021"));
        assert!(options.ignored.contains("nologo"));
        assert!(options.ignored.contains("optimize"));
        assert_eq!(options.sources, vec!["A.cs", "B.cs"]);
    }

    #[test]
    fn rejects_unknown_option() {
        let mut options = Options::default();
        let mut diagnostics = Vec::new();
        parse_options(
            &["-frobnicate:yes".to_string()],
            &mut options,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "CS2007");
        assert!(diagnostics[0].to_string().contains("frobnicate"));
    }

    #[test]
    fn rejects_invalid_langversion() {
        let mut options = Options::default();
        let mut diagnostics = Vec::new();
        parse_options(
            &["-langversion:banana".to_string()],
            &mut options,
            &mut diagnostics,
        );
        assert_eq!(diagnostics[0].code, "CS1617");
        assert!(is_known_langversion("latest"));
        assert!(is_known_langversion("7.3"));
        assert!(is_known_langversion("9"));
    }

    #[test]
    fn expands_nested_response_files() {
        let directory = tempfile::tempdir().unwrap();
        let inner = directory.path().join("inner.rsp");
        let outer = directory.path().join("outer.rsp");
        fs::write(&inner, "B.cs\n-out:patch.dll\n").unwrap();
        fs::write(
            &outer,
            format!("# generated by Unity\n\"A.cs\"\n\"@{}\"\n", inner.display()),
        )
        .unwrap();
        let mut diagnostics = Vec::new();
        let expanded = expand_arguments(&[format!("@{}", outer.display())], &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            expanded,
            vec![
                "A.cs".to_string(),
                "B.cs".to_string(),
                "-out:patch.dll".to_string()
            ]
        );
    }

    #[test]
    fn detects_response_file_cycle() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.rsp");
        let second = directory.path().join("second.rsp");
        fs::write(&first, format!("@{}\n", second.display())).unwrap();
        fs::write(&second, format!("@{}\n", first.display())).unwrap();
        let mut diagnostics = Vec::new();
        let expanded = expand_arguments(&[format!("@{}", first.display())], &mut diagnostics);
        assert!(expanded.is_empty());
        assert_eq!(diagnostics[0].code, "CD2330008");
    }

    #[test]
    fn reports_missing_response_file() {
        let mut diagnostics = Vec::new();
        let expanded = expand_arguments(&["@does-not-exist.rsp".to_string()], &mut diagnostics);
        assert!(expanded.is_empty());
        assert_eq!(diagnostics[0].code, "CD2330007");
    }

    #[test]
    fn maps_offsets_to_one_based_line_and_column() {
        let source = "class A {\n    void M( { }\n}";
        let offset = source.find("void").unwrap();
        assert_eq!(line_column(source, offset), (2, 5));
        assert_eq!(line_column(source, 0), (1, 1));
    }

    #[test]
    fn derives_output_name_from_first_source() {
        let options = Options {
            sources: vec!["Assets/Scripts/Player.cs".to_string()],
            ..Options::default()
        };
        assert_eq!(options.output_path(), PathBuf::from("Player.dll"));
    }

    #[test]
    fn rejects_non_library_target() {
        let mut options = Options {
            target: Some("exe".to_string()),
            sources: vec!["A.cs".to_string()],
            ..Options::default()
        };
        let mut diagnostics = Vec::new();
        let mut frontend = None;
        assert!(compile_assembly(&options, &mut diagnostics, &mut frontend).is_none());
        assert_eq!(diagnostics[0].code, "CD2330002");
        options.target = Some("library".to_string());
    }
}
