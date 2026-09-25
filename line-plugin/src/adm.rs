//! Stubs for Namco's ADM display library. Ported from src/adm.rs, except
//! admInitDevicei (no GLFW here) and admCreateWindowi, which is where the
//! test ends.

use crate::{boot, line};
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::Ordering;

extern "C" fn adm_version() -> *const c_char {
	c"WanganArcadeLoader 0.1".as_ptr()
}

#[allow(non_snake_case)]
#[repr(C)]
#[derive(Default)]
struct AdmChooseMode {
	ident: [u8; 4], // MOCF
	unk_0x04: u32,
	unk_0x08: u32,
	unk_0x0C: u32,
	unk_0x10: u32,
	unk_0x14: u32,
	width: u32,
	height: u32,
	refresh: u32,
}

extern "C" fn adm_config() -> *const *const AdmChooseMode {
	let adm = AdmChooseMode {
		ident: *b"MOCF",
		refresh: 60,
		..Default::default()
	};
	Box::leak(Box::new(Box::leak(Box::new(adm)) as *const AdmChooseMode))
}

extern "C" fn adm_fb_config() -> *const u8 {
	Box::leak(Box::new(0))
}

unsafe extern "C" fn adm_window() -> *const c_void {
	log!("REACHED admCreateWindowi — GO");
	log!(
		"summary: {} hooks, {} missing, {} failed; hasp_login called: {}; cl_main returned: {}; system() calls: {}; align shim: {}",
		line::HOOKED.load(Ordering::Relaxed),
		line::MISSING.load(Ordering::Relaxed),
		line::FAILED.load(Ordering::Relaxed),
		boot::HASP_LOGIN_CALLED.load(Ordering::Relaxed),
		boot::CL_MAIN_RETURNED.load(Ordering::Relaxed),
		boot::SYSTEM_CALLS.load(Ordering::Relaxed),
		if boot::align_shim_is_aligned() { "aligned" } else { "misaligned" },
	);

	// Exit through msys so LINE's buffered console output is flushed.
	let exit = line::resolve_stub("exit");
	if exit.is_null() {
		std::process::exit(0);
	}
	let exit: unsafe extern "C" fn(c_int) -> ! = std::mem::transmute(exit);
	exit(0)
}

pub unsafe fn init() {
	for symbol in [
		"admvt_setup",
		"admShutdown",
		"admGetNumDevices",
		"admInitDevicei",
		"admModeConfigi",
		"admCreateScreeni",
		"admCreateGraphicsContext",
		"admDisplayScreen",
		"admMakeContextCurrent",
		"admSwapInterval",
		"admCursorAttribi",
		"admGetDeviceAttribi",
		"admSwapBuffers",
		"admSetMonitorGamma",
	] {
		line::hook_symbol(symbol, boot::adachi as *const c_void);
	}
	line::hook_symbol("admGetString", adm_version as *const c_void);
	line::hook_symbol("admChooseModeConfigi", adm_config as *const c_void);
	line::hook_symbol("admChooseFBConfigi", adm_fb_config as *const c_void);
	line::hook_symbol("admCreateWindowi", adm_window as *const c_void);
}
