use std::{env, fs, process::ExitCode};
fn main() -> ExitCode {
    let mut args: Vec<_> = env::args().skip(1).collect();
    if args == ["edit"] {
        return luma::edits::serve(std::io::stdin().lock(), std::io::stdout().lock())
            .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS);
    }
    if let [action, flag, root] = args.as_slice() {
        if action == "edit" && flag == "--workspace" {
            return luma::edits::serve_workspace(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                Some(std::path::Path::new(root)),
            )
            .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS);
        }
    }
    if args.first().map(String::as_str) == Some("ir") {
        if args.len() != 2 {
            eprintln!("Usage: luma ir <file.luma>");
            return ExitCode::from(2);
        }
        let document = luma::structural::analyze(&args[1]);
        println!(
            "{}",
            document
                .to_json()
                .expect("serializable structural document")
        );
        return ExitCode::from(u8::from(!document.valid));
    }
    if args.first().map(String::as_str) == Some("run") {
        if args.len() != 2 {
            eprintln!("Usage: luma run <file.luma>");
            return ExitCode::from(2);
        }
        return match luma::runtime::run(&args[1]) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(failure) => {
                for (file, d) in failure.diagnostics {
                    eprintln!(
                        "{file}:bytes {}..{}: {}: {}",
                        d.span.start, d.span.end, d.code, d.message
                    );
                }
                ExitCode::from(if failure.runtime { 3 } else { 1 })
            }
        };
    }
    if args == ["version"] || args == ["--version"] {
        println!(
            "luma {} (0.1D2 standard library)",
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) != Some("check") || args.len() < 2 {
        eprintln!(
            "Usage: luma check [--syntax-only] [--graph output.json] <file.luma> | luma run <file.luma> | luma ir <file.luma> | luma edit | luma version"
        );
        return ExitCode::from(2);
    }
    args.remove(0);
    let syntax = args.first().map(String::as_str) == Some("--syntax-only");
    if syntax {
        args.remove(0);
    }
    let graph = if args.first().map(String::as_str) == Some("--graph") && args.len() >= 3 {
        args.remove(0);
        Some(args.remove(0))
    } else {
        None
    };
    if args.is_empty() || (graph.is_some() && (syntax || args.len() != 1)) {
        return ExitCode::from(2);
    }
    eprintln!(
        "{}; no execution.",
        if syntax {
            "Syntax-only check"
        } else {
            "Bounded semantic check (0.1C)"
        }
    );
    let mut failed = false;
    for path in args {
        if syntax {
            match fs::read_to_string(&path) {
                Ok(source) => match luma::parse(&source) {
                    Ok(_) => println!("{path}: syntax OK"),
                    Err(d) => {
                        let (line, col) = luma::position(&source, d.span.start);
                        eprintln!("{path}:{line}:{col}: {}: {}", d.code, d.message);
                        failed = true
                    }
                },
                Err(e) => {
                    eprintln!("{path}: {e}");
                    failed = true
                }
            }
        } else {
            let report = luma::semantic::check(&path);
            for (file, d) in &report.diagnostics {
                eprintln!(
                    "{file}:bytes {}..{}: {}: {}",
                    d.span.start, d.span.end, d.code, d.message
                )
            }
            failed |= !report.diagnostics.is_empty();
            if report.diagnostics.is_empty() {
                println!("{path}: bounded semantics OK")
            }
            if let Some(output) = &graph {
                if let Err(e) =
                    fs::write(output, serde_json::to_string_pretty(&report.graph).unwrap())
                {
                    eprintln!("{output}: {e}");
                    failed = true
                }
            }
        }
    }
    ExitCode::from(u8::from(failed))
}
