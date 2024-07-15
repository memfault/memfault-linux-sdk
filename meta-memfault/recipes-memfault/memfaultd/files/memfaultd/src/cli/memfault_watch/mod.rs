//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::File,
    io::{stderr, stdout, BufReader, BufWriter},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};

use argh::{FromArgs, TopLevelCommand};
use buffer::{monitor_and_buffer, STOP_THREADS};
use chrono::Local;
use eyre::{eyre, Result};
use flate2::{write::ZlibEncoder, Compression};
use log::{error, info, trace, LevelFilter};

use crate::{
    cli::{init_logger, MemfaultdClient},
    config::Config,
    mar::{CompressionAlgorithm, Metadata},
};

use super::memfaultctl::WrappedArgs;

mod buffer;

#[derive(FromArgs)]
/// A command line utility to run a specified command and send its output to our backend
struct MemfaultWatchArgs {
    /// use configuration file
    #[argh(option, short = 'c')]
    config_file: Option<String>,
    /// verbose output
    #[argh(switch, short = 'V')]
    verbose: bool,

    /// read in positional command argument
    #[argh(positional)]
    command: Vec<String>,
}

pub fn main() -> Result<()> {
    let args: MemfaultWatchArgs = from_env();
    let exit_status = run_from_args(args)?;
    std::process::exit(exit_status)
}

fn run_from_args(args: MemfaultWatchArgs) -> Result<i32> {
    init_logger(if args.verbose {
        LevelFilter::Trace
    } else {
        LevelFilter::Info
    });

    let config_path = args.config_file.as_ref().map(Path::new);
    let config = Config::read_from_system(config_path)?;

    // Set up log files
    let file_name = format!("mfw-log-{}", Local::now().to_rfc3339());
    let stdio_log_file_name = format!("{file_name}.zlib");
    let stdio_log_file = File::create(&stdio_log_file_name)
        .map_err(|_| eyre!("Failed to create output file on filesystem!"))?;

    let (command, additional_args) = args
        .command
        .split_first()
        .ok_or_else(|| eyre!("No command given!"))?;

    trace!("Running command: {:?}", command);

    let start = Instant::now();

    let mut child = Command::new(command)
        .args(additional_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| eyre!("Failed to run command! Does it exist in path?\nError: {e}"))?;

    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| eyre!("Failed to take stdout handle from child process."))?;
    let child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| eyre!("Failed to take stderr handle from child process."))?;

    let mut child_stdout_reader = BufReader::new(child_stdout);
    let mut child_stderr_reader = BufReader::new(child_stderr);

    let compression_encoder_stdio = ZlibEncoder::new(stdio_log_file, Compression::fast());

    let stdio_file_writer = Arc::new(Mutex::new(BufWriter::new(compression_encoder_stdio)));

    let child_stdout_writer = stdio_file_writer;
    let child_stderr_writer = child_stdout_writer.clone();

    let child_stdout_monitor = std::thread::spawn(move || {
        monitor_and_buffer(&mut child_stdout_reader, &mut stdout(), child_stdout_writer);
    });

    let child_stderr_monitor = std::thread::spawn(move || {
        monitor_and_buffer(&mut child_stderr_reader, &mut stderr(), child_stderr_writer);
    });

    let mut get_status = || match child.try_wait() {
        Ok(Some(status)) => {
            trace!("Command completed with status {status}!");
            Ok(ProcessStatus::Exited(status))
        }
        Ok(None) => Ok(ProcessStatus::Running),
        Err(e) => {
            error!("Failed to check command status! {e}");
            Err(e)
        }
    };
    let either_thread_still_running =
        || !child_stdout_monitor.is_finished() || !child_stderr_monitor.is_finished();

    // Check condition of process
    while matches!(get_status(), Ok(ProcessStatus::Running)) {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Now that the process is no longer running, send message to threads
    STOP_THREADS.store(true, Ordering::Relaxed);

    // While threads still cleaning up
    while either_thread_still_running() {
        std::thread::sleep(Duration::from_millis(100));
    }

    let child_stdout_monitor = child_stdout_monitor.join();
    let child_stderr_monitor = child_stderr_monitor.join();

    let status = match get_status() {
        Ok(ProcessStatus::Exited(status)) => status,
        _ => return Err(eyre!("Failed to retrieve exit status!")),
    };

    let exit_code = status
        .code()
        .ok_or_else(|| eyre!("Failed to retrieve exit status code!"))?;

    match (&child_stdout_monitor, &child_stderr_monitor) {
        (Ok(_), Ok(_)) => {
            trace!("Execution completed and monitor threads shut down.")
        }
        _ => {
            error!(
                "Error shutting down monitor threads. \n{:?} | {:?}",
                &child_stdout_monitor, &child_stderr_monitor
            )
        }
    }

    let duration = start.elapsed();

    trace!("Command completed in {} ms", duration.as_millis());

    let _metadata = Metadata::LinuxMemfaultWatch {
        cmdline: args.command,
        exit_code,
        duration,
        stdio_log_file_name,
        compression: CompressionAlgorithm::Zlib,
    };

    if !status.success() {
        info!("Command failed with exit code {exit_code}!");
        let client = MemfaultdClient::from_config(&config)
            .map_err(|report| eyre!("Failed to create Memfaultd client from config! {report}"))?;

        if client.notify_crash().is_err() {
            error!("Unable to contact memfaultd. Is it running?");
        }
    }

    Ok(exit_code)
}

/// Utilizes the WrappedArgs which provide version information
pub fn from_env<T: TopLevelCommand>() -> T {
    argh::from_env::<WrappedArgs<T>>().0
}

enum ProcessStatus {
    Running,
    Exited(ExitStatus),
}

use sealed_test::prelude::*;

#[sealed_test]
fn test_success_propagates() {
    let args: MemfaultWatchArgs = MemfaultWatchArgs {
        config_file: None,
        verbose: false,
        command: vec!["ls".into()],
    };

    assert_eq!(run_from_args(args).unwrap(), 0);
}

#[sealed_test]
fn test_error_propagates() {
    let args: MemfaultWatchArgs = MemfaultWatchArgs {
        config_file: None,
        verbose: false,
        command: vec!["bash".into(), "-c".into(), "exit 7".into()],
    };

    assert_eq!(run_from_args(args).unwrap(), 7);
}
