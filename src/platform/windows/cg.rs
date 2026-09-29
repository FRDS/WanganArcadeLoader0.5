//! Optional logging of the game's Cg calls, for diagnosing shader problems.
//!
//! The engine is a Static/Release build of Intrinsic Alchemy, so its own
//! reporting (`defaultReportLevel`, `printCompiledShaders` in alchemy.ini) is
//! compiled out and cannot be switched on. Since `OnDlSym` already intercepts
//! every Cg symbol the game resolves from cg.dll/cgGL.dll, wrapping a handful
//! of them is the only way to see which profile Cg picks on a given GPU and
//! why a shader program fails to compile.
//!
//! Enabled with `cg_log = true` in config.toml. Cg is cdecl on Windows, like
//! the game's Linux calls, so the wrappers are plain `extern "C"`.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};

type CgContext = *mut c_void;
type CgProgram = *mut c_void;

/// The context from cgCreateContext, so failures can be paired with the
/// compiler listing that explains them.
static CONTEXT: AtomicUsize = AtomicUsize::new(0);

static REAL_CREATE_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static REAL_CREATE_PROGRAM: AtomicUsize = AtomicUsize::new(0);
static REAL_GL_LOAD_PROGRAM: AtomicUsize = AtomicUsize::new(0);
static REAL_GL_LATEST_PROFILE: AtomicUsize = AtomicUsize::new(0);
static REAL_GL_SET_OPTIMAL_OPTIONS: AtomicUsize = AtomicUsize::new(0);

/// Looked up on demand rather than wrapped, so the wrappers can report errors
/// without the game having resolved these itself.
static REAL_GET_ERROR: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_ERROR_STRING: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_LAST_LISTING: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_PROFILE_STRING: AtomicUsize = AtomicUsize::new(0);

unsafe fn text(ptr: *const c_char) -> String {
	if ptr.is_null() {
		"(null)".to_string()
	} else {
		CStr::from_ptr(ptr).to_string_lossy().into_owned()
	}
}

/// "arbfp1 (6151)" — the name Cg gives a profile, plus the raw enum in case
/// the name lookup isn't available.
unsafe fn profile_name(profile: c_int) -> String {
	let slot = REAL_GET_PROFILE_STRING.load(Ordering::Relaxed);
	if slot == 0 {
		return format!("profile {profile}");
	}
	let f: unsafe extern "C" fn(c_int) -> *const c_char = std::mem::transmute(slot);
	format!("{} ({profile})", text(f(profile)))
}

/// Logs any pending Cg error together with the compiler listing, which is
/// where a failed shader compile actually explains itself.
unsafe fn report_error(what: &str) {
	let get_error = REAL_GET_ERROR.load(Ordering::Relaxed);
	if get_error == 0 {
		return;
	}
	let get_error: unsafe extern "C" fn() -> c_int = std::mem::transmute(get_error);
	let error = get_error();
	if error == 0 {
		return;
	}

	let described = match REAL_GET_ERROR_STRING.load(Ordering::Relaxed) {
		0 => format!("error {error}"),
		slot => {
			let f: unsafe extern "C" fn(c_int) -> *const c_char = std::mem::transmute(slot);
			format!("{} ({error})", text(f(error)))
		}
	};
	crate::wal_log!("cg: {what} failed: {described}");

	let listing = REAL_GET_LAST_LISTING.load(Ordering::Relaxed);
	let context = CONTEXT.load(Ordering::Relaxed);
	if listing == 0 || context == 0 {
		return;
	}
	let f: unsafe extern "C" fn(CgContext) -> *const c_char = std::mem::transmute(listing);
	let listing = text(f(context as CgContext));
	if listing != "(null)" && !listing.is_empty() {
		crate::wal_log!("cg: listing: {}", listing.trim_end());
	}
}

unsafe extern "C" fn create_context() -> CgContext {
	let slot = REAL_CREATE_CONTEXT.load(Ordering::Relaxed);
	let f: unsafe extern "C" fn() -> CgContext = std::mem::transmute(slot);
	let context = f();
	CONTEXT.store(context as usize, Ordering::Relaxed);
	crate::wal_log!("cg: cgCreateContext -> {context:p}");
	context
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn create_program(
	context: CgContext,
	program_type: c_int,
	source: *const c_char,
	profile: c_int,
	entry: *const c_char,
	args: *const *const c_char,
) -> CgProgram {
	let slot = REAL_CREATE_PROGRAM.load(Ordering::Relaxed);
	let f: unsafe extern "C" fn(
		CgContext,
		c_int,
		*const c_char,
		c_int,
		*const c_char,
		*const *const c_char,
	) -> CgProgram = std::mem::transmute(slot);
	let program = f(context, program_type, source, profile, entry, args);
	crate::wal_log!(
		"cg: cgCreateProgram entry={} profile={} -> {program:p}",
		text(entry),
		profile_name(profile),
	);
	if program.is_null() {
		// The source is the only way to tell which shader this was; the first
		// line usually carries the file name in a comment.
		let source = text(source);
		crate::wal_log!("cg: source began: {}", source.lines().next().unwrap_or(""));
	}
	report_error("cgCreateProgram");
	program
}

unsafe extern "C" fn gl_load_program(program: CgProgram) {
	let slot = REAL_GL_LOAD_PROGRAM.load(Ordering::Relaxed);
	let f: unsafe extern "C" fn(CgProgram) = std::mem::transmute(slot);
	f(program);
	report_error("cgGLLoadProgram");
}

unsafe extern "C" fn gl_latest_profile(profile_type: c_int) -> c_int {
	let slot = REAL_GL_LATEST_PROFILE.load(Ordering::Relaxed);
	let f: unsafe extern "C" fn(c_int) -> c_int = std::mem::transmute(slot);
	let profile = f(profile_type);
	crate::wal_log!(
		"cg: cgGLGetLatestProfile({profile_type}) -> {}",
		profile_name(profile)
	);
	profile
}

unsafe extern "C" fn gl_set_optimal_options(profile: c_int) {
	let slot = REAL_GL_SET_OPTIMAL_OPTIONS.load(Ordering::Relaxed);
	let f: unsafe extern "C" fn(c_int) = std::mem::transmute(slot);
	crate::wal_log!("cg: cgGLSetOptimalOptions({})", profile_name(profile));
	f(profile);
}

/// Remembers the helpers the wrappers call themselves, whether or not the game
/// ever resolves them. Call once per Cg DLL as it is loaded.
pub(super) unsafe fn prime(module: *mut c_void) {
	for (name, slot) in [
		("cgGetError", &REAL_GET_ERROR),
		("cgGetErrorString", &REAL_GET_ERROR_STRING),
		("cgGetLastListing", &REAL_GET_LAST_LISTING),
		("cgGetProfileString", &REAL_GET_PROFILE_STRING),
	] {
		// The core cg* helpers live in cg.dll only, so never let the call for
		// cgGL.dll overwrite a good address with null.
		let address = super::library_symbol(module, name) as usize;
		if address != 0 {
			slot.store(address, Ordering::Relaxed);
		}
	}
}

/// Our logging stand-in for `symbol`, given the real implementation, or None
/// to hand the game the real one unchanged.
pub(super) unsafe fn wrapper(symbol: &str, real: *mut c_void) -> Option<*mut c_void> {
	if real.is_null() {
		return None;
	}
	let (slot, wrapper) = match symbol {
		"cgCreateContext" => (&REAL_CREATE_CONTEXT, create_context as *const c_void),
		"cgCreateProgram" => (&REAL_CREATE_PROGRAM, create_program as *const c_void),
		"cgGLLoadProgram" => (&REAL_GL_LOAD_PROGRAM, gl_load_program as *const c_void),
		"cgGLGetLatestProfile" => (
			&REAL_GL_LATEST_PROFILE,
			gl_latest_profile as *const c_void,
		),
		"cgGLSetOptimalOptions" => (
			&REAL_GL_SET_OPTIMAL_OPTIONS,
			gl_set_optimal_options as *const c_void,
		),
		_ => return None,
	};
	slot.store(real as usize, Ordering::Relaxed);
	Some(wrapper as *mut c_void)
}
