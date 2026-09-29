//! The Windows half of `file_redirect`, plus logging of the files the game
//! opens.
//!
//! On Linux this is done by the `open`/`fopen` interposers in platform/linux.rs.
//! Under LINE there is no dynamic symbol interposition, so LINE's own stubs are
//! hooked by address instead, the same way `system` is in mod.rs.
//!
//! LINE already rewrites `/tmp/...` to `./tmp/...` itself (its
//! `fixPathIfNeeded` compares against "/tmp/" and formats with ".%s"), so this
//! deliberately does *not* repeat that rewrite — doing so would double the
//! prefix. Every other path LINE passes through unchanged, which is why an
//! absolute Linux path the game asks for lands outside LINE's whitelisted game
//! directory and fails. `file_redirect` in config.toml is how those get mapped
//! onto real files.

use super::line;
use crate::platform::call_game;
use crate::CONFIG;
use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::Mutex;

static ORIGINAL_OPEN: [AtomicPtrCell; 2] = [AtomicPtrCell::new(), AtomicPtrCell::new()];
static ORIGINAL_FOPEN: [AtomicPtrCell; 2] = [AtomicPtrCell::new(), AtomicPtrCell::new()];

/// A `*const ()` that can live in a `static`.
pub struct AtomicPtrCell(std::sync::atomic::AtomicUsize);

impl AtomicPtrCell {
	const fn new() -> Self {
		AtomicPtrCell(std::sync::atomic::AtomicUsize::new(0))
	}
	fn get(&self) -> *const () {
		self.0.load(std::sync::atomic::Ordering::Relaxed) as *const ()
	}
	fn set(&self, value: *const ()) {
		self.0
			.store(value as usize, std::sync::atomic::Ordering::Relaxed);
	}
}

/// Paths already reported as failing, so a game that polls for a missing file
/// in a loop doesn't bury the interesting line or slow loading to a crawl.
static REPORTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
const MAX_REPORTED: usize = 256;

/// True the first time this exact path fails.
fn first_failure(path: &str) -> bool {
	let Ok(mut reported) = REPORTED.lock() else {
		return false;
	};
	let reported = reported.get_or_insert_with(HashSet::new);
	if reported.len() >= MAX_REPORTED {
		return false;
	}
	reported.insert(path.to_string())
}

/// The replacement path and flag override for `path`, or None to pass the
/// caller's own pointer through untouched.
fn redirect(path: &str) -> Option<(String, Option<u32>)> {
	// LINE and msys share these stubs; never rewrite an already-Windows path.
	if path.contains(':') || path.starts_with('\\') {
		return None;
	}
	let redirects = unsafe { CONFIG.file_redirect.as_ref()? };
	let entry = redirects.iter().find(|entry| entry.from == path)?;
	Some((entry.to.clone(), entry.flags))
}

/// Reports the outcome, and returns the path actually used.
unsafe fn report(call: &str, original: &str, used: &str, ok: bool, result: String) {
	let redirected = if used == original {
		String::new()
	} else {
		format!(" -> \"{used}\"")
	};
	if !ok {
		if first_failure(used) {
			crate::wal_log!("{call}(\"{original}\"){redirected} = {result} FAILED");
		}
	} else if CONFIG.file_log {
		crate::wal_log!("{call}(\"{original}\"){redirected} = {result}");
	}
}

unsafe fn open_impl(
	slot: &AtomicPtrCell,
	call: &str,
	path: *const c_char,
	flags: c_int,
	mode: c_int,
) -> c_int {
	let original = slot.get();
	if original.is_null() || path.is_null() {
		return -1;
	}
	let name = CStr::from_ptr(path).to_string_lossy().into_owned();
	let (used, flags) = match redirect(&name) {
		Some((to, override_flags)) => (to, override_flags.unwrap_or(flags as u32) as c_int),
		None => (name.clone(), flags),
	};
	// A CString only for a redirect; otherwise the caller's pointer is reused,
	// so a path with an interior NUL can't turn a working open into a failure.
	let replacement = (used != name).then(|| CString::new(used.as_str()).ok()).flatten();
	let argument = match &replacement {
		Some(c) => c.as_ptr(),
		None => path,
	};
	let result: c_int = call_game(original, (argument, flags, mode));
	report(call, &name, &used, result >= 0, result.to_string());
	result
}

unsafe fn fopen_impl(
	slot: &AtomicPtrCell,
	call: &str,
	path: *const c_char,
	mode: *const c_char,
) -> *mut c_void {
	let original = slot.get();
	if original.is_null() || path.is_null() {
		return std::ptr::null_mut();
	}
	let name = CStr::from_ptr(path).to_string_lossy().into_owned();
	let used = match redirect(&name) {
		Some((to, _)) => to,
		None => name.clone(),
	};
	let replacement = (used != name).then(|| CString::new(used.as_str()).ok()).flatten();
	let argument = match &replacement {
		Some(c) => c.as_ptr(),
		None => path,
	};
	let result: *mut c_void = call_game(original, (argument, mode));
	report(call, &name, &used, !result.is_null(), format!("{result:p}"));
	result
}

unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
	open_impl(&ORIGINAL_OPEN[0], "open", path, flags, mode)
}

unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
	open_impl(&ORIGINAL_OPEN[1], "open64", path, flags, mode)
}

unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut c_void {
	fopen_impl(&ORIGINAL_FOPEN[0], "fopen", path, mode)
}

unsafe extern "C" fn fopen64(path: *const c_char, mode: *const c_char) -> *mut c_void {
	fopen_impl(&ORIGINAL_FOPEN[1], "fopen64", path, mode)
}

pub(super) unsafe fn init() {
	// LINE may resolve open64 to the same address as open. MinHook refuses a
	// second hook on one target and LINE drops the error code, which would
	// leave a null trampoline and make every call through it fail.
	let mut hooked: Vec<*mut c_void> = Vec::new();
	let mut installed = Vec::new();

	for (name, detour, slot) in [
		("open", open as *const c_void, &ORIGINAL_OPEN[0]),
		("open64", open64 as *const c_void, &ORIGINAL_OPEN[1]),
		("fopen", fopen as *const c_void, &ORIGINAL_FOPEN[0]),
		("fopen64", fopen64 as *const c_void, &ORIGINAL_FOPEN[1]),
	] {
		let stub = line::resolve_stub(name);
		if stub.is_null() {
			continue;
		}
		if hooked.contains(&stub) {
			crate::wal_log!("files: {name} shares a stub with an earlier hook, skipped");
			continue;
		}
		match line::hook(stub, detour) {
			Some(original) => {
				slot.set(original as *const ());
				hooked.push(stub);
				installed.push(name);
			}
			None => crate::wal_log!("files: failed to hook {name}() at {stub:p}"),
		}
	}
	if installed.is_empty() {
		crate::wal_log!("files: no open/fopen stub could be hooked, file logging is off");
	} else {
		crate::wal_log!("files: hooked {}", installed.join(", "));
	}
}

#[cfg(test)]
mod tests {
	use super::redirect;

	#[test]
	fn leaves_windows_paths_alone() {
		assert!(redirect("C:\\game\\main").is_none());
		assert!(redirect("\\\\?\\C:\\game").is_none());
	}

	#[test]
	fn leaves_unmatched_paths_alone() {
		assert!(redirect("data/graphic/car.cg").is_none());
	}
}
