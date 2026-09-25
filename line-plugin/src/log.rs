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
	let line = format!("[wal_line] {msg}\n");
	let mut stdout = std::io::stdout();
	_ = stdout.write_all(line.as_bytes());
	_ = stdout.flush();
	if let Ok(mut log) = LOG_FILE.lock() {
		if let Some(file) = log.as_mut() {
			_ = file.write_all(line.as_bytes());
			_ = file.flush();
		}
	}
}

#[macro_export]
macro_rules! log {
	($($arg:tt)*) => {
		$crate::log::write(&format!($($arg)*))
	};
}
