//! Run a short root script with sudo after the TUI has released the terminal.

use std::io::{self, Write};
use std::process::{Command, Stdio};

use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{self, LeaveAlternateScreen};
use crossterm::ExecutableCommand;

use crate::fstab::{self, CredWrite};

pub enum SudoJob {
    ApplyFstab {
        contents: String,
        creds: Option<CredWrite>,
        mount: Option<String>,
    },
    Mount(String),
    Umount(String),
}

pub fn run(job: &SudoJob) -> Result<String, String> {
    let script = match job {
        SudoJob::ApplyFstab {
            contents,
            creds,
            mount,
        } => {
            let backup = format!(
                "/etc/fstab.bak.{}",
                chrono_stamp()
            );
            fstab::render_script(&backup, contents, creds.as_ref(), mount.as_deref())
        }
        SudoJob::Mount(target) => format!("set -eu\nmount -- {}\n", fstab::sh_quote(target)),
        SudoJob::Umount(target) => format!("set -eu\numount -- {}\n", fstab::sh_quote(target)),
    };
    pause()?;
    let result = run_script(&script);
    // The caller resumes the terminal even when sudo fails.
    result
}

pub fn pause() -> Result<(), String> {
    let mut out = io::stdout();
    let _ = terminal::disable_raw_mode();
    let _ = out.execute(LeaveAlternateScreen);
    let _ = out.execute(DisableMouseCapture);
    let _ = out.execute(crossterm::cursor::Show);
    let _ = writeln!(
        out,
        "disktui needs administrator permission. sudo will ask for your password."
    );
    let _ = out.flush();
    Ok(())
}

pub fn resume(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    io::stdout().execute(terminal::EnterAlternateScreen)?;
    io::stdout().execute(crossterm::cursor::Hide)?;
    terminal.clear()?;
    Ok(())
}

fn run_script(script: &str) -> Result<String, String> {
    let mut child = Command::new("sudo")
        .arg("sh")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start sudo: {e}"))?;
    {
        let stdin = child.stdin.as_mut().ok_or("sudo stdin closed")?;
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("could not send the script to sudo: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("sudo failed: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("sudo exited with {}", output.status)
        } else {
            err
        })
    }
}

fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}
