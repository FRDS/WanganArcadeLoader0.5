//! Windows: the game runs under LINE (https://github.com/axylol/line), a
//! Windows loader for the game's Linux ELF files. LINE loads this DLL as its
//! plugin, calls the exported On* functions and hands us a function table for
//! symbol lookup and hooking (MinHook). libc calls go to LINE's stubs, which
//! we hook by address where the game needs different behaviour.

mod crash;
mod line;
pub mod log;
mod system;

use crate::*;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

pub const OPENAL_LIBRARY: &str = "soft_oal.dll";

#[link(name = "winmm")]
extern "system" {
	fn timeBeginPeriod(period: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
	fn LoadLibraryA(name: *const c_char) -> *mut c_void;
	fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
	fn GetLastError() -> u32;
}

/// Called by LINE right after loading the plugin. LINE's linker doesn't exist
/// yet, so only the function table, the log and the crash handler are set up.
#[no_mangle]
pub unsafe extern "C" fn OnInitialize(version: c_int, func_tables: *const *const c_void) {
	log::open("wal_3dxp.log");
	std::panic::set_hook(Box::new(|info| {
		crate::wal_log!("PANIC: {info}");
	}));
	crate::wal_log!("OnInitialize v{version}");
	if !line::init(func_tables) {
		crate::wal_log!("OnInitialize: invalid LINE function table");
		return;
	}
	crash::install();
	unbuffer_msys_stdout();
}

/// Called by LINE before each module's init code runs, and before main's entry.
#[no_mangle]
pub unsafe extern "C" fn OnPreExecute(lib_name: *const c_char, base_address: *mut c_void) {
	if lib_name.is_null() {
		return;
	}
	let name = CStr::from_ptr(lib_name).to_string_lossy();
	crate::wal_log!("OnPreExecute {name} (base {base_address:p})");
	if name == "main" {
		crate::init();
	}
}

static CG: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static CG_GL: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

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
	let Some(module) = try_load_library(dll) else {
		return false;
	};
	crate::wal_log!("OnDlOpen {name} -> {dll}");
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
	// Cg's Windows API is cdecl, like the game's Linux calls.
	*result = GetProcAddress(handle, symbol);
	true
}

/// LINE's own messages go through msys stdio, which is fully buffered when
/// redirected to a file and lost on a crash. Make it unbuffered.
unsafe fn unbuffer_msys_stdout() {
	const IONBF: c_int = 2; // newlib's _IONBF
	let stdout = line::resolve_stub("stdout") as *const *mut c_void;
	let setvbuf = line::resolve_stub("setvbuf");
	if stdout.is_null() || setvbuf.is_null() || (*stdout).is_null() {
		return;
	}
	let setvbuf: unsafe extern "C" fn(*mut c_void, *mut c_char, c_int, usize) -> c_int =
		std::mem::transmute(setvbuf);
	setvbuf(*stdout, null_mut(), IONBF, 0);
}

pub unsafe fn get_symbol(symbol: &str) -> *mut () {
	line::dl_sym(symbol) as *mut ()
}

pub unsafe fn hook(address: *mut (), func: *const ()) -> *const () {
	match line::hook(address as *mut c_void, func as *const c_void) {
		Some(original) => original as *const (),
		None => std::ptr::null(),
	}
}

/// LINE maps the game read-write-execute, so a plain copy is enough.
pub unsafe fn write_memory(address: *mut (), data: &[u8]) {
	line::patch_bytes(address as *mut u8, data);
}

unsafe fn try_load_library(name: &str) -> Option<*mut c_void> {
	let name_c = CString::new(name).unwrap();
	let module = LoadLibraryA(name_c.as_ptr());
	if module.is_null() {
		crate::wal_log!("LoadLibraryA({name}) failed, error {}", GetLastError());
		None
	} else {
		Some(module)
	}
}

/// Loads a DLL; panics if it's missing.
pub unsafe fn load_library(name: &str) -> *mut c_void {
	try_load_library(name).unwrap_or_else(|| panic!("{name} not found next to line.exe"))
}

pub unsafe fn library_symbol(module: *mut c_void, symbol: &str) -> *mut c_void {
	let symbol = CString::new(symbol).unwrap();
	GetProcAddress(module, symbol.as_ptr())
}

/// Exits through msys so LINE's stdio is flushed.
pub fn exit(code: i32) -> ! {
	unsafe {
		let exit = line::resolve_stub("exit");
		if !exit.is_null() {
			let exit: unsafe extern "C" fn(c_int) -> ! = std::mem::transmute(exit);
			exit(code);
		}
	}
	std::process::exit(code)
}

/// Windows-only setup, run at the start of crate::init.
pub unsafe fn init() {
	// 1 ms timer resolution, for the frame limiter's sleeps.
	timeBeginPeriod(1);

	// The game reads the USB device list from /proc to find its dongle.
	match line::signature("2f 70 72 6f 63 2f 62 75 73") {
		Some(address) => line::patch_bytes(address, b"tmp/usb-devices\0"),
		None => crate::wal_log!("warning: /proc/bus/usb/devices string not found"),
	}

	let system_stub = line::resolve_stub("system");
	if line::hook(system_stub, system::system as *const c_void).is_none() {
		crate::wal_log!("failed to hook system() at {system_stub:p}");
	}

	// Windows can't set interface addresses; the game only needs the call to succeed.
	hook::hook_symbol("_ZN5clNet19setInterfaceAddressEv", adachi as *const ());
}

/// Networking stays off unless an address is configured explicitly.
pub fn network_available() -> bool {
	unsafe { CONFIG.local_ip.is_some() }
}

pub unsafe fn load_plugins(_version: &GameVersion) {
	if std::path::Path::new("plugins").is_dir() {
		crate::wal_log!("plugins/ ignored: plugins aren't supported on Windows yet");
	}
}
