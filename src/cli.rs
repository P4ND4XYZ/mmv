//! Public CLI contract. Parsing and output conversion never initialize GPUI.
use std::{ffi::OsString, fmt::Display, path::Path};

use clap::{Parser, Subcommand};

use crate::app::{ExternalOperation, ScrollDirection};

#[derive(Debug, Parser)]
#[command(
    name = "mmv",
    version,
    about = "Read Markdown and control the existing viewer",
    subcommand_precedence_over_arg = true,
    subcommand_negates_reqs = true,
    args_conflicts_with_subcommands = true
)]
struct Args {
    /// Shorthand for open FILE (use open for a file named like a command)
    #[arg(value_name = "FILE", required = true)]
    file: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open one UTF-8 Markdown file
    Open {
        file: std::path::PathBuf,
    },
    /// Scroll by a line, half page, or page
    Scroll {
        #[command(subcommand)]
        direction: Direction,
    },
    Top,
    Bottom,
    Back,
    Forward,
}

#[derive(Debug, Subcommand)]
enum Direction {
    Down,
    Up,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
}

/// Ready-to-print output, including help/version and usage errors.
#[derive(Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl From<clap::Error> for CliOutput {
    fn from(error: clap::Error) -> Self {
        let exit_code = error.exit_code() as u8;
        let message = error.to_string();
        if error.use_stderr() {
            Self {
                exit_code,
                stdout: String::new(),
                stderr: message,
            }
        } else {
            Self {
                exit_code,
                stdout: message,
                stderr: String::new(),
            }
        }
    }
}

/// `cwd` is the invoking CLI's absolute working directory, not the GUI's.
/// Resolve paths without requiring that the file exists; reading belongs to infra.
pub fn parse_from<I, T>(args: I, cwd: &Path) -> Result<ExternalOperation, CliOutput>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::try_parse_from(args).map_err(CliOutput::from)?;
    let command = args.command.unwrap_or_else(|| Command::Open {
        file: args.file.expect("clap requires FILE or a command"),
    });
    Ok(match command {
        Command::Open { file } => {
            let path = if file.is_absolute() {
                file
            } else {
                cwd.join(file)
            };
            // Do not let Windows drive-relative paths silently use this process's
            // per-drive cwd. Ordinary relative and rooted paths use the supplied cwd.
            if !cwd.is_absolute() || !path.is_absolute() {
                return Err(execution_output(Err(
                    "an absolute CLI working directory and a resolvable file path are required",
                )));
            }
            let path = std::path::absolute(path).map_err(|error| execution_output(Err(error)))?;
            ExternalOperation::Open(path)
        }
        Command::Scroll { direction } => ExternalOperation::Scroll(match direction {
            Direction::Down => ScrollDirection::Down,
            Direction::Up => ScrollDirection::Up,
            Direction::HalfDown => ScrollDirection::HalfDown,
            Direction::HalfUp => ScrollDirection::HalfUp,
            Direction::PageDown => ScrollDirection::PageDown,
            Direction::PageUp => ScrollDirection::PageUp,
        }),
        Command::Top => ExternalOperation::Top,
        Command::Bottom => ExternalOperation::Bottom,
        Command::Back => ExternalOperation::Back,
        Command::Forward => ExternalOperation::Forward,
    })
}

/// Call only for a completed operation/read (never for OperationResult::Pending).
pub fn execution_output(outcome: Result<(), impl Display>) -> CliOutput {
    match outcome {
        Ok(()) => CliOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(error) => CliOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: format!("{error}\n"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> &'static Path {
        if cfg!(windows) {
            Path::new(r"C:\cli workspace")
        } else {
            Path::new("/cli workspace")
        }
    }

    #[test]
    fn explicit_and_shorthand_open_resolve_from_the_supplied_cli_directory() {
        for file in ["notes/日本語 memo.md", "missing.md", "./notes/file.md"] {
            let explicit = parse_from(["mmv", "open", file], cwd()).unwrap();
            assert_eq!(explicit, parse_from(["mmv", file], cwd()).unwrap());
            assert_eq!(explicit, ExternalOperation::Open(cwd().join(file)));
        }
        let absolute = cwd().join("absolute 日本語.md");
        assert_eq!(
            parse_from(
                [OsString::from("mmv"), absolute.clone().into_os_string()],
                &cwd().join("other")
            )
            .unwrap(),
            ExternalOperation::Open(absolute)
        );
    }

    #[test]
    fn parses_every_scroll_and_navigation_operation() {
        for (name, direction) in [
            ("down", ScrollDirection::Down),
            ("up", ScrollDirection::Up),
            ("half-down", ScrollDirection::HalfDown),
            ("half-up", ScrollDirection::HalfUp),
            ("page-down", ScrollDirection::PageDown),
            ("page-up", ScrollDirection::PageUp),
        ] {
            assert_eq!(
                parse_from(["mmv", "scroll", name], cwd()).unwrap(),
                ExternalOperation::Scroll(direction)
            );
        }
        for (name, operation) in [
            ("top", ExternalOperation::Top),
            ("bottom", ExternalOperation::Bottom),
            ("back", ExternalOperation::Back),
            ("forward", ExternalOperation::Forward),
        ] {
            assert_eq!(parse_from(["mmv", name], cwd()).unwrap(), operation);
        }
    }

    #[test]
    fn command_named_files_require_explicit_open() {
        for name in ["open", "scroll", "top", "bottom", "back", "forward", "help"] {
            assert_eq!(
                parse_from(["mmv", "open", name], cwd()).unwrap(),
                ExternalOperation::Open(cwd().join(name))
            );
        }
        assert_eq!(
            parse_from(["mmv", "open", "--", "--help"], cwd()).unwrap(),
            ExternalOperation::Open(cwd().join("--help"))
        );
    }

    #[test]
    fn usage_errors_use_stderr_and_exit_two() {
        for args in [
            vec!["mmv"],
            vec!["mmv", "open"],
            vec!["mmv", "scroll"],
            vec!["mmv", "scroll", "sideways"],
            vec!["mmv", "--unknown"],
            vec!["mmv", "a.md", "b.md"],
            vec!["mmv", "open", "a.md", "b.md"],
            vec!["mmv", "top", "extra"],
            vec!["mmv", "back", "--unknown"],
        ] {
            let output = parse_from(args.clone(), cwd()).unwrap_err();
            assert_eq!(output.exit_code, 2, "{args:?}");
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
        }
    }

    #[test]
    fn help_and_version_use_stdout_and_exit_zero_without_a_cwd_or_gui() {
        for args in [
            vec!["mmv", "--help"],
            vec!["mmv", "-h"],
            vec!["mmv", "help"],
            vec!["mmv", "--version"],
            vec!["mmv", "open", "--help"],
            vec!["mmv", "scroll", "--help"],
        ] {
            let output = parse_from(args, Path::new("")).unwrap_err();
            assert_eq!(output.exit_code, 0);
            assert!(!output.stdout.is_empty());
            assert!(output.stderr.is_empty());
        }
        let help = parse_from(["mmv", "--help"], cwd()).unwrap_err();
        assert!(help.stdout.contains("Usage: mmv"));
        for command in ["open", "scroll", "top", "bottom", "back", "forward"] {
            assert!(help.stdout.contains(command));
        }
        assert!(
            parse_from(["mmv", "--version"], cwd())
                .unwrap_err()
                .stdout
                .contains(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn completed_execution_is_silent_on_success_and_reports_failure_with_exit_one() {
        assert_eq!(
            execution_output(Ok::<_, &str>(())),
            CliOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new()
            }
        );
        for error in ["file not found", "connection failed", "timed out"] {
            assert_eq!(
                execution_output(Err(error)),
                CliOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("{error}\n")
                }
            );
        }
    }
}
