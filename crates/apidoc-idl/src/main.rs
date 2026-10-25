//! CLI: `apidoc-idl proto --in <file> --out <file> [--package <name>]`
//!      `apidoc-idl thrift --in <file> --out <file>`

use apidoc_idl::{proto, thrift_idl, MAX_INPUT};
use std::fs;
use std::process::ExitCode;

const USAGE: &str = "usage:
  apidoc-idl proto  --in <file> --out <file> [--package <name>]
  apidoc-idl thrift --in <file> --out <file>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("apidoc-idl: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.first().ok_or(USAGE)?;
    let mut input = None;
    let mut output = None;
    let mut package = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1).ok_or_else(|| format!("{flag} needs a value"))?;
        match flag {
            "--in" => input = Some(value),
            "--out" => output = Some(value),
            "--package" => package = Some(value.as_str()),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
        i += 2;
    }
    let input = input.ok_or("missing --in")?;
    let output = output.ok_or("missing --out")?;

    let size = fs::metadata(input).map_err(|e| format!("{input}: {e}"))?.len();
    if size > MAX_INPUT as u64 {
        return Err(format!("{input}: {size} bytes, the limit is 1 MiB"));
    }
    let source = fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
    let generated = match command.as_str() {
        "proto" => proto::generate(&source, package)?,
        "thrift" => {
            if package.is_some() {
                return Err("--package only applies to the proto command".to_string());
            }
            thrift_idl::generate(&source)?
        }
        other => return Err(format!("unknown command `{other}`\n{USAGE}")),
    };
    fs::write(output, generated).map_err(|e| format!("{output}: {e}"))
}
