//! Go/no-go test for running WanganArcadeLoader as a Rust plugin for LINE
//! (https://github.com/axylol/line) on Windows. It boots the game up to
//! admCreateWindowi, logs the result and exits. See README.md.

#![allow(non_snake_case, clippy::missing_safety_doc)]

#[cfg(not(all(windows, target_arch = "x86")))]
compile_error!("wal_line only builds for i686-pc-windows-gnu (a 32-bit LINE plugin)");

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

#[macro_use]
pub mod log;

mod adm;
mod al;
mod boot;
mod card;
mod crash;
mod line;

#[link(name = "kernel32")]
extern "system" {
	fn LoadLibraryA(name: *const c_char) -> *mut c_void;
	fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
	fn GetLastError() -> u32;
}

pub(crate) unsafe fn load_library(name: &str) -> *mut c_void {
	let name_c = CString::new(name).unwrap();
	let module = LoadLibraryA(name_c.as_ptr());
	if module.is_null() {
		log!("LoadLibraryA({name}) failed, error {}", GetLastError());
	}
	module
}

pub(crate) unsafe fn get_proc_address(module: *mut c_void, name: &CStr) -> *mut c_void {
	GetProcAddress(module, name.as_ptr())
}

static CG: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static CG_GL: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// Called by LINE right after loading the plugin. LINE's linker doesn't exist
/// yet, so only the function table is stored here.
#[no_mangle]
pub unsafe extern "C" fn OnInitialize(version: c_int, func_tables: *const *const c_void) {
	log::open("wal_line.log");
	std::panic::set_hook(Box::new(|info| {
		log!("PANIC: {info}");
	}));

	log!(
		"OnInitialize v{version} (align shim: {})",
		if boot::align_shim_is_aligned() { "aligned" } else { "MISALIGNED diagnostic build" }
	);
	if !line::init(func_tables) {
		log!("OnInitialize: invalid LINE function table");
		return;
	}
	crash::install();
	unbuffer_msys_stdout();
}

/// LINE's own messages (loaded modules, symbols it can't link) go through msys
/// stdio, which is fully buffered when redirected to a file and lost on a
/// crash. Make it unbuffered. LINE's stubs are ready before plugins load.
unsafe fn unbuffer_msys_stdout() {
	const IONBF: c_int = 2; // newlib's _IONBF
	let stdout = line::resolve_stub("stdout") as *const *mut c_void;
	let setvbuf = line::resolve_stub("setvbuf");
	if stdout.is_null() || setvbuf.is_null() || (*stdout).is_null() {
		log!("msys stdout not found, LINE's messages stay buffered");
		return;
	}
	let setvbuf: unsafe extern "C" fn(*mut c_void, *mut c_char, c_int, usize) -> c_int =
		std::mem::transmute(setvbuf);
	if setvbuf(*stdout, null_mut(), IONBF, 0) != 0 {
		log!("setvbuf on msys stdout failed");
	}
}

/// Called by LINE before each module's init code runs, and before main's entry.
#[no_mangle]
pub unsafe extern "C" fn OnPreExecute(lib_name: *const c_char, base_address: *mut c_void) {
	let name = if lib_name.is_null() {
		String::from("<null>")
	} else {
		CStr::from_ptr(lib_name).to_string_lossy().into_owned()
	};
	log!("OnPreExecute {name} (base {base_address:p})");
	if name == "main" {
		boot::init();
	}
}

/// Redirects the game's Linux Cg libraries to the Windows Cg Toolkit DLLs.
/// Returning true means `result` is used instead of LINE's own loader.
#[no_mangle]
pub unsafe extern "C" fn OnDlOpen(lib_name: *const c_char, result: *mut *mut c_void) -> bool {
	if lib_name.is_null() || result.is_null() {
		return false;
	}
	let name = CStr::from_ptr(lib_name).to_string_lossy();
	let (dll, slot) = match name.as_ref() {
		"libCg.so" => ("cg.dll", &CG),
		"libCgGL.so" => ("cgGL.dll", &CG_GL),
		_ => return false,
	};
	let module = load_library(dll);
	if module.is_null() {
		return false;
	}
	log!("OnDlOpen {name} -> {dll}");
	slot.store(module, Ordering::Relaxed);
	*result = module;
	true
}

#[no_mangle]
pub unsafe extern "C" fn OnDlSym(
	handle: *mut c_void,
	symbol: *const c_char,
	result: *mut *mut c_void,
) -> bool {
	if handle.is_null() || symbol.is_null() || result.is_null() {
		return false;
	}
	if handle != CG.load(Ordering::Relaxed) && handle != CG_GL.load(Ordering::Relaxed) {
		return false;
	}
	*result = get_proc_address(handle, CStr::from_ptr(symbol));
	true
}
