use std::{env, fs, process::ExitCode};
fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    if args == ["version"] || args == ["--version"] {
        println!("luma {} (syntax frontend)", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) != Some("check") || args.len() < 2 {
        eprintln!("Usage: luma check <file.luma>... | luma version\ncheck is syntax-only; no semantic analysis or execution.");
        return ExitCode::from(2);
    }
    eprintln!(
        "Syntax-only check: names, types, imports and effects are not analyzed; no execution."
    );
    let mut failed = false;
    for path in &args[1..] {
        match fs::read_to_string(path) {
            Ok(source) => match luma::parse(&source) {
                Ok(_) => println!("{path}: syntax OK"),
                Err(d) => {
                    let (line, col) = luma::position(&source, d.span.start);
                    eprintln!(
                        "{path}:{line}:{col}: {}: {} [bytes {}..{}]",
                        d.code, d.message, d.span.start, d.span.end
                    );
                    failed = true;
                }
            },
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
            }
        }
    }
    ExitCode::from(u8::from(failed))
}
