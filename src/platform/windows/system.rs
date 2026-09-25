//! LINE's system() stub only prints the command, and there is no shell, so
//! the commands the game relies on are emulated here (ported from
//! line-wal0.5's jmp_system).

use std::ffi::{c_char, c_int, CStr};

/// Writes the sorted list of files in `dir` whose name contains `filter` to
/// tmp/find.txt, like the `find ... | sort > /tmp/find.txt` the game runs.
fn find_to_file(dir: &str, filter: &str) {
	let mut names: Vec<String> = match std::fs::read_dir(dir) {
		Ok(entries) => entries
			.filter_map(|entry| entry.ok())
			.map(|entry| entry.file_name().to_string_lossy().into_owned())
			.filter(|name| name.contains(filter))
			.collect(),
		Err(err) => {
			crate::wal_log!("system: can't open {dir}: {err}");
			Vec::new()
		}
	};
	names.sort();
	let list: String = names.iter().map(|name| format!("{dir}/{name}\n")).collect();
	if let Err(err) = std::fs::write("tmp/find.txt", list) {
		crate::wal_log!("system: can't write tmp/find.txt: {err}");
	}
}

fn copy_targets() {
	let entries = match std::fs::read_dir("data/target") {
		Ok(entries) => entries,
		Err(err) => {
			crate::wal_log!("system: can't open data/target: {err}");
			return;
		}
	};
	for entry in entries.filter_map(|entry| entry.ok()) {
		let name = entry.file_name().to_string_lossy().into_owned();
		if !name.contains(".target.gz") {
			continue;
		}
		let dest = format!("tmp/data/target/{name}");
		if let Err(err) = std::fs::copy(entry.path(), &dest) {
			crate::wal_log!("system: can't copy {name} to {dest}: {err}");
		}
	}
}

// Replaces LINE's system() stub, which only prints the command. There is no
// shell, so the commands the game relies on are emulated here.
pub(super) unsafe extern "C" fn system(command: *const c_char) -> c_int {
	if command.is_null() {
		return 0;
	}
	let command = CStr::from_ptr(command).to_string_lossy();
	crate::wal_log!("system({command})");

	const FINDS: [(&str, &str, &str); 6] = [
		(
			"find /tmp/data/target/ -type f -name \"*.target.gz\"  -type f | sort >/tmp/find.txt",
			"tmp/data/target",
			".target.gz",
		),
		(
			"find data/target/us -type f -name \"*.target.gz\"  -type f | sort >/tmp/find.txt",
			"data/target/us",
			".target.gz",
		),
		(
			"find data/target/jp -type f -name \"*.target.gz\"  -type f | sort >/tmp/find.txt",
			"data/target/jp",
			".target.gz",
		),
		(
			"find /tmp/data/ranking/ -type f -name \"*.rank\"  -type f | sort >/tmp/find.txt",
			"tmp/data/ranking",
			".rank",
		),
		(
			"find /tmp/data/maxicoin/ -type f -name \"*.maxicoin\"  -type f | sort >/tmp/find.txt",
			"tmp/data/maxicoin",
			".maxicoin",
		),
		(
			"find /tmp/data/joinstar/ -type f -name \"*.joinstar\"  -type f | sort >/tmp/find.txt",
			"tmp/data/joinstar",
			".joinstar",
		),
	];
	if let Some((_, dir, filter)) = FINDS.iter().find(|(cmd, _, _)| *cmd == command) {
		find_to_file(dir, filter);
	} else if command == "cp -f data/target/*.target.gz /tmp/data/target/ 2>/dev/null" {
		copy_targets();
	} else {
		crate::wal_log!("system: unhandled command, ignored");
	}
	0
}
