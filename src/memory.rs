use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitCode},
};

const HELP: &str = "Usage: nayuta memory [--output NEW_FILE] -- COMMAND [ARG ...]
       nayuta memory --input PERF_DATA

Linux perf memory-access sampling. Requires perf and supported CPU/kernel events.
--output  Save recording (default: nayuta-memory.data); existing files are refused.
--input   Report an existing perf mem recording without running a command.

Reports sampled load/store events by source line and symbol, not exact access counts.
Build with debug information for source locations. Sampling bias depends on the CPU.
Target arguments after -- are passed directly, without a shell.";

#[derive(Debug, PartialEq)]
enum Mode {
    Record {
        output: PathBuf,
        command: Vec<OsString>,
    },
    Report(PathBuf),
    Help,
}

fn parse(args: Vec<OsString>) -> Result<Mode, String> {
    let mut args = args.into_iter();
    let mut output = None;
    let mut input = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(Mode::Help);
        }
        if arg == "--" {
            let command: Vec<_> = args.collect();
            if command.is_empty() || input.is_some() {
                return Err("expected a command after --; --input cannot run a command".into());
            }
            return Ok(Mode::Record {
                output: output.unwrap_or_else(|| PathBuf::from("nayuta-memory.data")),
                command,
            });
        }
        let slot = if arg == "--output" {
            &mut output
        } else if arg == "--input" {
            &mut input
        } else {
            return Err(format!("unknown memory option: {}", arg.to_string_lossy()));
        };
        if slot.is_some() {
            return Err("duplicate option".into());
        }
        *slot = Some(PathBuf::from(args.next().ok_or("missing option value")?));
    }
    match (input, output) {
        (Some(path), None) => Ok(Mode::Report(path)),
        _ => Err("specify -- COMMAND or --input PERF_DATA".into()),
    }
}

fn report_command(path: &std::path::Path) -> Command {
    let mut command = Command::new("perf");
    command
        .args([
            "mem",
            "report",
            "--stdio",
            "--show-nr-samples",
            "--sort=srcline,symbol",
            "-i",
        ])
        .arg(path);
    command
}

fn execute(mode: Mode) -> Result<(), String> {
    if mode == Mode::Help {
        println!("{HELP}");
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        return Err(
            "memory profiling requires Linux; copy perf.data to a Linux host to report it".into(),
        );
    }
    let path = match mode {
        Mode::Record { output, command } => {
            // Reserve the destination atomically so perf never overwrites user data.
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .map_err(|e| format!("cannot create {}: {e}", output.display()))?;
            eprintln!("Recording memory samples to {}", output.display());
            let status = Command::new("perf")
                .args(["mem", "record", "-o"])
                .arg(&output)
                .arg("--")
                .args(command)
                .status()
                .map_err(|e| {
                    format!(
                        "cannot run perf: {e}; install Linux perf. Recording path: {}",
                        output.display()
                    )
                })?;
            if !status.success() {
                return Err(format!(
                    "perf mem record failed ({status}); check perf permissions and CPU/kernel memory sampling support. Partial recording: {}",
                    output.display()
                ));
            }
            output
        }
        Mode::Report(path) => path,
        Mode::Help => unreachable!(),
    };
    eprintln!(
        "Memory-access samples, not exact counts. Unknown source locations require debug information. Event sampling may favor high-latency loads; percentages are not cache-miss rates."
    );
    let status = report_command(&path)
        .status()
        .map_err(|e| format!("cannot run perf report: {e}"))?;
    if !status.success() {
        return Err(format!(
            "perf mem report failed ({status}); recording retained at {}",
            path.display()
        ));
    }
    Ok(())
}

pub fn run(args: Vec<OsString>) -> ExitCode {
    match parse(args).and_then(execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nayuta memory: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn preserves_target_arguments() {
        assert_eq!(
            parse(args(&[
                "--output",
                "sample data",
                "--",
                "./app",
                "a b",
                "--input"
            ]))
            .unwrap(),
            Mode::Record {
                output: "sample data".into(),
                command: args(&["./app", "a b", "--input"])
            }
        );
    }
    #[test]
    fn validates_modes() {
        assert_eq!(
            parse(args(&["--input", "perf.data"])).unwrap(),
            Mode::Report("perf.data".into())
        );
        for invalid in [
            vec![],
            vec!["--"],
            vec!["--output"],
            vec!["--input", "a", "--", "app"],
            vec!["--input", "a", "--output", "b"],
        ] {
            assert!(parse(args(&invalid)).is_err());
        }
    }
    #[test]
    fn report_requests_source_locations_and_sample_counts() {
        let command = report_command(std::path::Path::new("recording"));
        let arguments: Vec<_> = command.get_args().collect();
        assert!(arguments.contains(&std::ffi::OsStr::new("--show-nr-samples")));
        assert!(arguments.contains(&std::ffi::OsStr::new("--sort=srcline,symbol")));
    }
}
