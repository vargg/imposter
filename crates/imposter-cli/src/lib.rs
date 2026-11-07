//! The command line. `imposter send` runs one request from a collection file without a
//! window and without the personal profile, through the same engine as Send
//! (`imposter_app::executor::run`), and prints a redacted result as JSON. The usage text,
//! the README and the agent skill are drawn from one table ([`SEND_FLAGS`]), so they cannot
//! drift from what the parser accepts.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;

use imposter_core::run_result::RunOutcome;

pub mod mcp;
pub mod send;

pub use send::{InputArg, SendArgs};

/// A flag of `imposter send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    pub name: &'static str,
    /// The value it takes, as the usage names it.
    pub value: &'static str,
    pub help: &'static str,
}

pub const SEND_FLAGS: [Flag; 5] = [
    Flag {
        name: "--env",
        value: "NAME",
        help: "Use the file's environment NAME; without it, only globals and collection variables apply.",
    },
    Flag {
        name: "--input",
        value: "VAR=PROCESS_VAR",
        help: "Set variable VAR from the process variable PROCESS_VAR, above every scope. Repeatable.",
    },
    Flag {
        name: "--secret",
        value: "VAR=PROCESS_VAR",
        help: "As --input, and remove its value from the result and error output. Repeatable.",
    },
    Flag {
        name: "--out",
        value: "FILE",
        help: "Also write the result to FILE; History opens it (⋯ → Open result file…).",
    },
    Flag {
        name: "--max-body",
        value: "BYTES",
        help: "Show at most BYTES of each body in the result (default 65536; 0 shows none).",
    },
];

/// What each outcome means to a caller, beside its exit status.
pub fn outcome_help(outcome: RunOutcome) -> &'static str {
    match outcome {
        RunOutcome::Passed => "a response arrived and every check and script test passed",
        RunOutcome::Failed => "a check or script test failed",
        RunOutcome::NotSent => {
            "nothing was sent: the file, request path, environment, an input or the request is wrong"
        }
        RunOutcome::TransportError => "no response: connection, TLS or timeout",
        RunOutcome::ScriptFailed => "a script failed or was stopped",
        RunOutcome::NotChecked => "a check could not be decided (cut body, bad path), none failed",
        RunOutcome::NoChecks => {
            "a response arrived but the request has no checks: nothing verified"
        }
        RunOutcome::Cancelled => "interrupted (Ctrl+C)",
    }
}

pub const USAGE_STATUS: u8 = 2;
pub const USAGE_HELP: &str = "the command line was wrong, or the result file could not be written";

/// Every exit status with its name and meaning, in numeric order.
pub fn exit_statuses() -> Vec<(u8, &'static str, &'static str)> {
    let mut statuses: Vec<_> = RunOutcome::ALL
        .iter()
        .map(|o| (o.exit_status(), o.as_str(), outcome_help(*o)))
        .chain([(USAGE_STATUS, "usage", USAGE_HELP)])
        .collect();
    statuses.sort_by_key(|(status, _, _)| *status);
    statuses
}

pub fn version() -> String {
    format!("imposter {}", env!("CARGO_PKG_VERSION"))
}

/// The help text, from [`SEND_FLAGS`] and [`RunOutcome::ALL`].
pub fn usage() -> String {
    let mut text = String::from(
        "Usage:\n  imposter                       open the app\n  imposter send <FILE> <REQUEST> [OPTIONS]\n\n\
         Run one request from a collection file (.imposter.json) without a window and without\n\
         your personal data. REQUEST is its path of names, e.g. \"Users/Create user\" (write a\n\
         \"/\" inside a name as \"\\/\"). The result is printed as JSON on stdout, a summary on\n\
         stderr.\n\nOptions:\n",
    );
    for flag in SEND_FLAGS {
        let left = format!("{} {}", flag.name, flag.value);
        // Writing to a String cannot fail.
        let _ = writeln!(text, "  {left:<26} {}", flag.help);
    }
    text.push_str("\nExit status:\n");
    for (status, name, help) in exit_statuses() {
        let _ = writeln!(text, "  {status:<4} {name:<16} {help}");
    }
    text.push_str(
        "\nOther commands:\n  imposter skill                 print instructions for a coding agent\n  imposter mcp <FILE> [--env NAME] [--input|--secret VAR=PROCESS_VAR]... [--max-body BYTES]\n                                 experimental: an MCP server over stdio for one file\n  imposter --help | --version\n",
    );
    text
}

/// The README's option table rows, generated from [`SEND_FLAGS`].
pub fn readme_flag_rows() -> Vec<String> {
    SEND_FLAGS
        .iter()
        .map(|f| format!("| `{} {}` | {} |", f.name, f.value, f.help))
        .collect()
}

/// The README's exit status table rows, generated from [`exit_statuses`].
pub fn readme_status_rows() -> Vec<String> {
    exit_statuses()
        .into_iter()
        .map(|(status, name, help)| format!("| {status} | `{name}` | {help} |"))
        .collect()
}

/// The skill text for a coding agent, printed by `imposter skill`. Its option, exit status
/// and operator lists come from the same tables as the parser and the help.
pub fn skill() -> String {
    let table = |header: &str, rows: Vec<String>| {
        let columns = header.matches('|').count() - 1;
        let mut text = format!("{header}\n|{}\n", "---|".repeat(columns));
        for row in rows {
            text.push_str(&row);
            text.push('\n');
        }
        text.trim_end().to_owned()
    };
    let operators: Vec<String> = imposter_core::check::Op::ALL
        .iter()
        .filter_map(|op| serde_json::to_value(op).ok())
        .filter_map(|value| value.as_str().map(|name| format!("`{name}`")))
        .collect();
    include_str!("skill.md")
        .replace(
            "{{options}}",
            &table("| Option | Meaning |", readme_flag_rows()),
        )
        .replace(
            "{{statuses}}",
            &table("| Status | Outcome | Meaning |", readme_status_rows()),
        )
        .replace("{{operators}}", &operators.join(", "))
}

/// What the binary should do with its arguments.
pub fn wants_command_line(args: &[OsString]) -> bool {
    args.first().is_some_and(|first| first != "--smoke-test")
}

/// The process's surroundings, passed in so tests can supply their own.
pub struct Io<'a> {
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
    /// Reads one named process variable; nothing else of the environment is read.
    pub var: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Skill,
    Send(SendArgs),
    /// The experimental MCP server: a FILE and the run's environment and inputs.
    Mcp(SendArgs),
}

fn parse(args: Vec<OsString>) -> Result<Command, String> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let first = match parser.next().map_err(|e| e.to_string())? {
        None => return Err("a command is needed".into()),
        Some(Long("help") | Short('h')) => return Ok(Command::Help),
        Some(Long("version") | Short('V')) => return Ok(Command::Version),
        Some(Value(value)) => value.string().map_err(|e| e.to_string())?,
        Some(other) => return Err(other.unexpected().to_string()),
    };
    match first.as_str() {
        "help" => Ok(Command::Help),
        "skill" => match parser.next().map_err(|e| e.to_string())? {
            None => Ok(Command::Skill),
            Some(other) => Err(other.unexpected().to_string()),
        },
        "send" => parse_send(&mut parser, false),
        "mcp" => parse_send(&mut parser, true),
        other => Err(format!("unknown command {other:?}")),
    }
}

fn input(value: OsString, secret: bool) -> Result<InputArg, String> {
    let text = value
        .into_string()
        .map_err(|_| "inputs must be valid UTF-8".to_owned())?;
    match text.split_once('=') {
        Some((variable, from)) if !variable.is_empty() && !from.is_empty() => Ok(InputArg {
            variable: variable.to_owned(),
            from: from.to_owned(),
            secret,
        }),
        _ => Err(format!("{text:?} is not VAR=PROCESS_VAR")),
    }
}

/// `send`'s arguments, or with `mcp` the server's: a FILE without a REQUEST (the agent
/// chooses it per call) and no `--out`.
fn parse_send(parser: &mut lexopt::Parser, mcp: bool) -> Result<Command, String> {
    use lexopt::prelude::*;
    let mut positional: Vec<OsString> = Vec::new();
    let mut args = SendArgs::default();
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        let value = |parser: &mut lexopt::Parser| parser.value().map_err(|e| e.to_string());
        match arg {
            Long("help") | Short('h') => return Ok(Command::Help),
            Long("env") => {
                args.environment = Some(
                    value(parser)?
                        .into_string()
                        .map_err(|_| "--env must be valid UTF-8".to_owned())?,
                );
            }
            Long("input") => args.inputs.push(input(value(parser)?, false)?),
            Long("secret") => args.inputs.push(input(value(parser)?, true)?),
            Long("out") if !mcp => args.out = Some(PathBuf::from(value(parser)?)),
            Long("max-body") => {
                args.max_body = value(parser)?
                    .into_string()
                    .ok()
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(|| "--max-body takes a number of bytes".to_owned())?;
            }
            Value(value) => positional.push(value),
            other => return Err(other.unexpected().to_string()),
        }
    }
    let mut positional = positional.into_iter();
    if mcp {
        let (Some(file), None) = (positional.next(), positional.next()) else {
            return Err("mcp takes a FILE".into());
        };
        args.file = PathBuf::from(file);
        return Ok(Command::Mcp(args));
    }
    let (Some(file), Some(request), None) =
        (positional.next(), positional.next(), positional.next())
    else {
        return Err("send takes a FILE and a REQUEST".into());
    };
    args.file = PathBuf::from(file);
    args.request = request
        .into_string()
        .map_err(|_| "REQUEST must be valid UTF-8".to_owned())?;
    Ok(Command::Send(args))
}

/// Run the command line and return the process exit status. Ctrl+C cancels a running send,
/// which is reported as cancelled.
pub fn main(args: Vec<OsString>, io: &mut Io<'_>) -> u8 {
    // A broken stdout or stderr (a closed pipe) cannot be reported anywhere else.
    let command = match parse(args) {
        Ok(command) => command,
        Err(message) => {
            let _ = writeln!(
                io.stderr,
                "error: {message}\nRun imposter --help for usage."
            );
            return USAGE_STATUS;
        }
    };
    match command {
        Command::Help => {
            let _ = io.stdout.write_all(usage().as_bytes());
            0
        }
        Command::Version => {
            let _ = writeln!(io.stdout, "{}", version());
            0
        }
        Command::Skill => {
            let _ = io.stdout.write_all(skill().as_bytes());
            0
        }
        Command::Send(args) => {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = writeln!(io.stderr, "error: cannot start: {error}");
                    return USAGE_STATUS;
                }
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            let interrupt = cancel.clone();
            let result = runtime.block_on(async {
                tokio::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        interrupt.cancel();
                    }
                });
                send::send(&args, io.var, cancel).await
            });
            send::report(&args, &result, io.stdout, io.stderr)
        }
        Command::Mcp(args) => {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = writeln!(io.stderr, "error: cannot start: {error}");
                    return USAGE_STATUS;
                }
            };
            let (lines, received) = tokio::sync::mpsc::unbounded_channel();
            // Reading stdin blocks, so a thread feeds the server; it ends with stdin.
            std::thread::spawn(move || {
                for line in std::io::stdin().lines() {
                    let Ok(line) = line else { break };
                    if lines.send(line).is_err() {
                        break;
                    }
                }
            });
            let _ = writeln!(
                io.stderr,
                "imposter mcp (experimental): serving {} over stdio",
                args.file.display()
            );
            match runtime.block_on(mcp::serve(received, io.stdout, &args, io.var)) {
                Ok(()) => 0,
                Err(error) => {
                    let _ = writeln!(io.stderr, "error: stdout closed: {error}");
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn send_arguments_parse_in_any_order() {
        let parsed = parse(args(&[
            "send",
            "--env",
            "Dev",
            "api.imposter.json",
            "--secret",
            "token=API_TOKEN",
            "Users/Create user",
            "--input=id=USER_ID",
            "--max-body",
            "0",
            "--out",
            "result.json",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Send(SendArgs {
                file: "api.imposter.json".into(),
                request: "Users/Create user".into(),
                environment: Some("Dev".into()),
                inputs: vec![
                    InputArg {
                        variable: "token".into(),
                        from: "API_TOKEN".into(),
                        secret: true
                    },
                    InputArg {
                        variable: "id".into(),
                        from: "USER_ID".into(),
                        secret: false
                    },
                ],
                out: Some("result.json".into()),
                max_body: 0,
            })
        );
    }

    #[test]
    fn wrong_command_lines_are_usage_errors() {
        for (line, message) in [
            (&[][..], "a command is needed"),
            (&["sned"][..], "unknown command \"sned\""),
            (&["send", "a.json"][..], "send takes a FILE and a REQUEST"),
            (
                &["send", "a", "b", "c"][..],
                "send takes a FILE and a REQUEST",
            ),
            (
                &["send", "a", "b", "--input", "x"][..],
                "\"x\" is not VAR=PROCESS_VAR",
            ),
            (
                &["send", "a", "b", "--input", "=Y"][..],
                "\"=Y\" is not VAR=PROCESS_VAR",
            ),
            (
                &["send", "a", "b", "--max-body", "lots"][..],
                "--max-body takes a number of bytes",
            ),
            (
                &["send", "a", "b", "--env"][..],
                "missing argument for option '--env'",
            ),
            (&["send", "a", "b", "--loud"][..], "invalid option '--loud'"),
            (&["--loud"][..], "invalid option '--loud'"),
        ] {
            assert_eq!(parse(args(line)), Err(message.to_owned()), "{line:?}");
        }
        assert_eq!(parse(args(&["mcp"])), Err("mcp takes a FILE".to_owned()));
        assert_eq!(
            parse(args(&["mcp", "a.json", "--out", "x"])),
            Err("invalid option '--out'".to_owned())
        );
        assert!(matches!(
            parse(args(&["mcp", "a.json", "--env", "Dev"])),
            Ok(Command::Mcp(SendArgs {
                environment: Some(_),
                ..
            }))
        ));
        assert_eq!(parse(args(&["help"])), Ok(Command::Help));
        assert_eq!(parse(args(&["send", "--help"])), Ok(Command::Help));
        assert_eq!(parse(args(&["-V"])), Ok(Command::Version));
    }

    #[test]
    fn the_window_opens_without_arguments_and_for_the_smoke_test() {
        assert!(!wants_command_line(&[]));
        assert!(!wants_command_line(&args(&["--smoke-test", "http://x"])));
        assert!(wants_command_line(&args(&["send"])));
        assert!(wants_command_line(&args(&["--bogus"])));
    }

    fn readme_table(header: &str) -> Vec<String> {
        include_str!("../../../README.md")
            .lines()
            .skip_while(|l| *l != header)
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_readme_lists_exactly_these_options_and_statuses() {
        assert_eq!(readme_table("| Option | Meaning |"), readme_flag_rows());
        assert_eq!(
            readme_table("| Status | Outcome | Meaning |"),
            readme_status_rows()
        );
    }

    #[test]
    fn the_skill_is_filled_from_the_tables() {
        let skill = skill();
        for placeholder in ["{{options}}", "{{statuses}}", "{{operators}}"] {
            assert!(!skill.contains(placeholder), "{placeholder}");
        }
        for row in readme_flag_rows().iter().chain(&readme_status_rows()) {
            assert!(skill.contains(row.as_str()), "{row}");
        }
        for op in imposter_core::check::Op::ALL {
            let name = serde_json::to_value(op).unwrap();
            assert!(
                skill.contains(&format!("`{}`", name.as_str().unwrap())),
                "{op:?}"
            );
        }
        assert_eq!(parse(args(&["skill"])), Ok(Command::Skill));
        assert_eq!(
            parse(args(&["skill", "--write"])),
            Err("invalid option '--write'".to_owned())
        );
    }

    #[test]
    fn usage_lists_every_flag_and_exit_status() {
        let usage = usage();
        for flag in SEND_FLAGS {
            assert!(usage.contains(flag.name), "{}", flag.name);
        }
        for outcome in RunOutcome::ALL {
            assert!(usage.contains(outcome.as_str()), "{outcome:?}");
        }
    }
}
