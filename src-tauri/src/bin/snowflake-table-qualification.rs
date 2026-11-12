//! Deliberately offline: this binary has no execute command or async runtime.

use std::io::{self, Write};

fn main() {
    #[cfg(unix)]
    let result = joinery_core::drivers::snowflake_table_qualification::run_offline_command(
        std::env::args_os().skip(1),
    );
    #[cfg(not(unix))]
    let result: Result<serde_json::Value, _> = Err(serde_json::json!({
        "failureCode": "unsupported_platform",
    }));

    let (value, success) = match result {
        Ok(value) => (value, true),
        Err(error) => (
            serde_json::to_value(error)
                .unwrap_or_else(|_| serde_json::json!({"failureCode": "offline_command_failed"})),
            false,
        ),
    };
    let output = if success {
        write_json(io::stdout().lock(), &value)
    } else {
        write_json(io::stderr().lock(), &value)
    };
    if !success || output.is_err() {
        std::process::exit(1);
    }
}

fn write_json(mut output: impl Write, value: &serde_json::Value) -> io::Result<()> {
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}
