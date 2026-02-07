//! Debug logging: file-based log output with rotation.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::config::Config;

const MAX_LOG_BYTES: u64 = 3 * 1024 * 1024; // 3MB

struct LogWriter {
    file: BufWriter<File>,
    path: PathBuf,
    bytes_written: u64,
    verbose: bool,
}

static LOG: OnceLock<Mutex<LogWriter>> = OnceLock::new();

/// Initialize the global logger. Call once after Config::load().
pub fn init(verbose: bool) {
    let Some(config_path) = Config::config_path() else {
        return;
    };
    let Some(config_dir) = config_path.parent() else {
        return;
    };
    let _ = fs::create_dir_all(config_dir);
    let log_path = config_dir.join("ump.log");

    // Check existing file size — rotate if already over limit
    if let Ok(meta) = fs::metadata(&log_path) {
        if meta.len() >= MAX_LOG_BYTES {
            rotate_file(&log_path);
        }
    }

    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(_) => return,
    };

    let bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);

    let writer = LogWriter {
        file: BufWriter::new(file),
        path: log_path,
        bytes_written,
        verbose,
    };

    let _ = LOG.set(Mutex::new(writer));
}

/// Return whether verbose (Info-level) logging is enabled.
pub fn is_verbose() -> bool {
    LOG.get()
        .and_then(|m| m.lock().ok())
        .map(|w| w.verbose)
        .unwrap_or(false)
}

/// Write a log line. Called by macros.
#[doc(hidden)]
pub fn write_log(level: &str, msg: &str) {
    let Some(lock) = LOG.get() else { return };
    let Ok(mut writer) = lock.lock() else { return };

    let now = chrono_now();
    let line = format!("{} [{}] {}\n", now, level, msg);
    let len = line.len() as u64;

    if writer.file.write_all(line.as_bytes()).is_ok() {
        let _ = writer.file.flush();
        writer.bytes_written += len;

        if writer.bytes_written >= MAX_LOG_BYTES {
            rotate(&mut writer);
        }
    }
}

/// Rotate: rename current log, open fresh file.
fn rotate(writer: &mut LogWriter) {
    let _ = writer.file.flush();

    rotate_file(&writer.path);

    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&writer.path)
    {
        writer.file = BufWriter::new(file);
        writer.bytes_written = 0;
    }
}

/// Rename a log file with timestamp suffix.
fn rotate_file(path: &PathBuf) {
    let now = chrono_now().replace(['-', ':', ' '], "");
    // "20260207123456" format — strip non-digits for filename
    let digits: String = now.chars().filter(|c| c.is_ascii_digit()).collect();
    let rotated = path.with_file_name(format!("ump_{}.log", digits));
    let _ = fs::rename(path, rotated);
}

/// Simple timestamp without external crate: YYYY-MM-DD HH:MM:SS (local time).
fn chrono_now() -> String {
    // Convert to local time via Windows API (only when d2d feature provides `windows` crate)
    #[cfg(feature = "d2d")]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;

        let st = unsafe { GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }

    #[cfg(not(feature = "d2d"))]
    {
        // Fallback: UTC
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = secs / 86400;
        let time = secs % 86400;
        let h = time / 3600;
        let m = (time % 3600) / 60;
        let sec = time % 60;
        // Simplified date from days since epoch
        let (y, mo, d) = days_to_ymd(days);
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, sec)
    }
}

#[cfg(not(feature = "d2d"))]
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simple Gregorian conversion from days since 1970-01-01
    let mut y = 1970;
    let mut rem = days;
    loop {
        let ylen = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if rem < ylen { break; }
        rem -= ylen;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0;
    for (i, &ml) in mdays.iter().enumerate() {
        if rem < ml { mo = i; break; }
        rem -= ml;
    }
    (y, (mo + 1) as u64, (rem + 1) as u64)
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::debug::write_log("ERROR", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::debug::write_log("WARN", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::debug::is_verbose() {
            $crate::debug::write_log("INFO", &format!($($arg)*))
        }
    };
}
