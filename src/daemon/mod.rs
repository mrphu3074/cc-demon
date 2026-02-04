use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::config::PathsConfig;

fn pid_file() -> PathBuf {
    PathsConfig::default().pid_file()
}

pub fn is_running() -> Result<bool> {
    let pid_path = pid_file();
    if !pid_path.exists() {
        return Ok(false);
    }

    let pid = read_pid()?;

    // Check if process is actually running
    #[cfg(unix)]
    {
        use nix::sys::signal;
        use nix::unistd::Pid;
        match signal::kill(Pid::from_raw(pid), None) {
            Ok(_) => Ok(true),
            Err(nix::errno::Errno::ESRCH) => {
                // Process doesn't exist, clean up stale PID file
                let _ = fs::remove_file(&pid_path);
                Ok(false)
            }
            Err(e) => Err(e).context("Failed to check process status"),
        }
    }

    #[cfg(not(unix))]
    {
        // On non-Unix, just trust the PID file
        Ok(true)
    }
}

pub fn read_pid() -> Result<i32> {
    let content = fs::read_to_string(pid_file()).context("Failed to read PID file")?;
    content
        .trim()
        .parse()
        .context("Invalid PID in PID file")
}

pub fn write_pid() -> Result<()> {
    let pid = std::process::id();
    let pid_path = pid_file();
    fs::create_dir_all(pid_path.parent().unwrap())?;
    fs::write(&pid_path, pid.to_string())?;
    Ok(())
}

pub fn remove_pid() -> Result<()> {
    let pid_path = pid_file();
    if pid_path.exists() {
        fs::remove_file(&pid_path)?;
    }
    Ok(())
}

pub fn stop_daemon(pid: i32) -> Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        signal::kill(Pid::from_raw(pid), Signal::SIGTERM)
            .context("Failed to send SIGTERM to daemon")?;
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!("Stopping daemon is only supported on Unix systems. Kill process {pid} manually.");
    }

    // Wait for process to exit
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !is_running().unwrap_or(false) {
            remove_pid()?;
            return Ok(());
        }
    }

    anyhow::bail!("Daemon did not stop within 3 seconds")
}

pub fn signal_reload(pid: i32) -> Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        signal::kill(Pid::from_raw(pid), Signal::SIGHUP)
            .context("Failed to send SIGHUP to daemon")?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        // On non-Unix, the daemon polls for changes
        Ok(())
    }
}

pub fn daemonize(with_gateway: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use daemonize::Daemonize;

        let paths = PathsConfig::default();
        let base = paths.base_dir();
        fs::create_dir_all(&base)?;
        fs::create_dir_all(paths.logs_dir())?;
        fs::create_dir_all(paths.output_dir())?;

        let stdout = fs::File::create(paths.logs_dir().join("demon.out"))?;
        let stderr = fs::File::create(paths.logs_dir().join("demon.err"))?;

        let daemonize = Daemonize::new()
            .pid_file(paths.pid_file())
            .working_directory(&base)
            .stdout(stdout)
            .stderr(stderr);

        daemonize.start().context("Failed to daemonize")?;

        // We're now in the daemon process
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let config = crate::config::DemonConfig::load()?;

            let scheduler_handle = tokio::spawn({
                let config = config.clone();
                async move {
                    if let Err(e) = crate::scheduler::run(config).await {
                        tracing::error!("Scheduler error: {e}");
                    }
                }
            });

            if with_gateway {
                tokio::spawn({
                    let config = config.clone();
                    async move {
                        if let Err(e) = crate::gateway::run(config).await {
                            tracing::error!("Gateway error: {e}");
                        }
                    }
                });
            }

            scheduler_handle.await?;
            Ok::<(), anyhow::Error>(())
        })?;
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!("Daemonization is only supported on Unix. Use --foreground mode on Windows.");
    }

    Ok(())
}
