//! LINE's plugin function table (see axylol/line plugin.cpp), and helpers
//! ported from line-wal0.5's shared/line.cpp.

use std::ffi::{c_char, c_void, CString};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

#[repr(C)]
struct FuncTable {
	_dl_open: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
	dl_sym: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
	hook: Option<unsafe extern "C" fn(*mut c_void, *const c_void, *mut *mut c_void)>,
	// Always returns NULL in LINE, never used.
	_get_module_by_name: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
	get_module_start: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
	get_module_size: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
	get_module_by_base_handle: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
	resolve_stub: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
}

static TABLE: OnceLock<FuncTable> = OnceLock::new();

pub static HOOKED: AtomicU32 = AtomicU32::new(0);
pub static MISSING: AtomicU32 = AtomicU32::new(0);
pub static FAILED: AtomicU32 = AtomicU32::new(0);

pub unsafe fn init(func_tables: *const *const c_void) -> bool {
	if func_tables.is_null() {
		return false;
	}
	TABLE.set(std::ptr::read(func_tables as *const FuncTable)).is_ok()
}

fn table() -> &'static FuncTable {
	TABLE.get().expect("LINE function table not initialised")
}

/// Looks up a symbol in the loaded game modules, like dlsym(NULL, name).
pub unsafe fn dl_sym(name: &str) -> *mut c_void {
	let name = CString::new(name).unwrap();
	table().dl_sym.map_or(null_mut(), |f| f(null_mut(), name.as_ptr()))
}

/// Returns LINE's implementation of a libc function (its own stub, or msys-2.0.dll's).
pub unsafe fn resolve_stub(name: &str) -> *mut c_void {
	let name = CString::new(name).unwrap();
	table().resolve_stub.map_or(null_mut(), |f| f(name.as_ptr()))
}

/// Hooks `target` with MinHook through LINE and returns the trampoline to the
/// original. LINE drops MinHook's error code, so a null trampoline is the only
/// failure signal.
pub unsafe fn hook(target: *mut c_void, detour: *const c_void) -> Option<*mut c_void> {
	let hook = table().hook?;
	if target.is_null() {
		return None;
	}
	let mut original: *mut c_void = null_mut();
	hook(target, detour, &mut original);
	if original.is_null() {
		None
	} else {
		Some(original)
	}
}

pub unsafe fn hook_symbol(name: &str, detour: *const c_void) -> Option<*mut c_void> {
	let target = dl_sym(name);
	if target.is_null() {
		MISSING.fetch_add(1, Ordering::Relaxed);
		crate::log!("hook: symbol not found: {name}");
		return None;
	}
	match hook(target, detour) {
		Some(original) => {
			HOOKED.fetch_add(1, Ordering::Relaxed);
			Some(original)
		}
		None => {
			FAILED.fetch_add(1, Ordering::Relaxed);
			crate::log!("hook: failed to hook {name} at {target:p}");
			None
		}
	}
}

/// Parses "FF ?? 00" style patterns. `None` is a wildcard byte.
fn parse_pattern(pattern: &str) -> Option<Vec<Option<u8>>> {
	pattern
		.split_whitespace()
		.map(|byte| {
			if byte.starts_with('?') {
				Some(None)
			} else {
				u8::from_str_radix(byte, 16).ok().map(Some)
			}
		})
		.collect()
}

/// Start and size of the main module's first loaded segment (where its code is).
pub fn main_module_range() -> Option<(usize, usize)> {
	let table = TABLE.get()?;
	unsafe {
		let module = table.get_module_by_base_handle?(null_mut());
		if module.is_null() {
			return None;
		}
		let start = table.get_module_start?(module) as usize;
		let size = table.get_module_size?(module) as usize;
		(start != 0).then_some((start, size))
	}
}

/// Scans the main module for a byte pattern.
pub unsafe fn signature(pattern: &str) -> Option<*mut u8> {
	let table = table();
	let bytes = parse_pattern(pattern)?;
	if bytes.is_empty() {
		return None;
	}
	let module = table.get_module_by_base_handle?(null_mut());
	if module.is_null() {
		crate::log!("signature: main module not found");
		return None;
	}
	let start = table.get_module_start?(module) as *const u8;
	let size = table.get_module_size?(module) as usize;
	if start.is_null() || size < bytes.len() {
		return None;
	}
	let memory = std::slice::from_raw_parts(start, size);
	memory
		.windows(bytes.len())
		.position(|window| {
			window
				.iter()
				.zip(&bytes)
				.all(|(byte, expected)| expected.is_none_or(|expected| *byte == expected))
		})
		.map(|offset| start.add(offset) as *mut u8)
}

/// Writes bytes into game memory. LINE maps the game read-write-execute.
pub unsafe fn patch_bytes(address: *mut u8, data: &[u8]) {
	std::ptr::copy_nonoverlapping(data.as_ptr(), address, data.len());
}
