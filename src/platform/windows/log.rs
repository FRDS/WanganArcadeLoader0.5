use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn open(path: &str) {
	let file = File::create(path).ok();
	if let Ok(mut log) = LOG_FILE.lock() {
		*log = file;
	}
}

// Every line is flushed immediately so the log survives a crash.
pub fn write(msg: &str) {
	let line = format!("[wal_3dxp] {msg}\n");
	_ = std::io::stdout().write_all(line.as_bytes());
	if let Ok(mut log) = LOG_FILE.lock() {
		write_file(&mut log, &line);
	}
}

/// For the crash handler: never waits on the log lock, in case the crash
/// happened while it was held.
pub fn write_nonblocking(msg: &str) {
	let line = format!("[wal_3dxp] {msg}\n");
	_ = std::io::stdout().write_all(line.as_bytes());
	if let Ok(mut log) = LOG_FILE.try_lock() {
		write_file(&mut log, &line);
	}
}

fn write_file(log: &mut Option<File>, line: &str) {
	if let Some(file) = log.as_mut() {
		_ = file.write_all(line.as_bytes());
		_ = file.flush();
	}
}

/// Logs to stdout and wal_3dxp.log (Windows only).
#[macro_export]
macro_rules! wal_log {
	($($arg:tt)*) => {
		$crate::platform::log::write(&format!($($arg)*))
	};
}
