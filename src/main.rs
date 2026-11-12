extern crate gpui_kit as gpui;

mod app;
mod cli;
mod config;
mod core;
mod infra;
mod runtime;
mod ui;

use std::time::Instant;

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 2 && args[1] == infra::INTERNAL_GUI_ARGUMENT {
        return print_output(cli::execution_output(runtime::run_gui()));
    }
    // Help/version/usage are handled before any GUI or Windows resources exist.
    let cwd = std::env::current_dir();
    let operation = match cli::parse_from(args, cwd.as_deref().unwrap_or(std::path::Path::new("")))
    {
        Ok(operation) => operation,
        Err(output) => return print_output(output),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return print_output(cli::execution_output(Err(error.to_string()))),
    };
    // One CLI negotiation on this thread; never move its InstanceGuard to a worker.
    let result = runtime.block_on(async {
        let namespace = infra::Namespace::current_user().map_err(|error| error.to_string())?;
        // Only a cfg(test) build can isolate the actual process/main startup path.
        #[cfg(all(test, feature = "ui-tests"))]
        let namespace = infra::test_support::process_namespace().unwrap_or(namespace);
        let client = infra::connect_or_start(&namespace, &operation)
            .await
            .map_err(|error| error.to_string())?;
        // Independent of app generations. One invocation sends exactly one request.
        let id = app::ExternalRequestId(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        client
            .request(
                infra::Request { id, operation },
                infra::Deadline::response(Instant::now()),
            )
            .await
            .map_err(|error| error.to_string())?
            .outcome
    });
    print_output(cli::execution_output(result))
}

fn print_output(output: cli::CliOutput) -> std::process::ExitCode {
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    std::process::ExitCode::from(output.exit_code)
}
