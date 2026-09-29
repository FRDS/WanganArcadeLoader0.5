//! Linux: the loader is injected with LD_PRELOAD and runs from a constructor.
//! Symbols are looked up with dlsym and hooked with retour, and libc calls are
//! intercepted by exporting functions with the same names.

use crate::*;
use std::mem::transmute;

#[cfg(not(test))]
#[ctor::ctor]
unsafe fn ctor_init() {
	crate::init();
}

pub const OPENAL_LIBRARY: &str = "libopenal.so";
pub const CG_LIBRARY: &str = "libCg.so";

/// Opens a shared library, or None if it isn't there. Unlike `load_library`,
/// a missing one is not fatal.
pub unsafe fn try_load_library(name: &str) -> Option<*mut c_void> {
	let name = CString::new(name).ok()?;
	let module = dlopen(name.as_ptr(), RTLD_LAZY);
	(!module.is_null()).then_some(module)
}

pub unsafe fn get_symbol(symbol: &str) -> *mut () {
	let symbol = CString::new(symbol).unwrap();
	let module = dlopen(std::ptr::null(), RTLD_LAZY);
	let address = dlsym(module, symbol.as_ptr());
	dlclose(module);
	address as *mut ()
}

pub unsafe fn hook(address: *mut (), func: *const ()) -> *const () {
	let Ok(hook) = retour::RawDetour::new(address, func) else {
		return std::ptr::null();
	};
	let Ok(_) = hook.enable() else {
		return std::ptr::null();
	};
	let trampoline = hook.trampoline() as *const ();
	std::mem::forget(hook);
	trampoline
}

pub unsafe fn write_memory(address: *mut (), data: &[u8]) {
	region::protect(address, data.len(), region::Protection::READ_WRITE_EXECUTE).unwrap();
	std::ptr::copy_nonoverlapping(data.as_ptr(), address as *mut u8, data.len());
}

/// Opens a shared library; panics with dlerror's message if it can't.
pub unsafe fn load_library(name: &str) -> *mut c_void {
	let name_c = CString::new(name).unwrap();
	let module = dlopen(name_c.as_ptr(), RTLD_LAZY);
	if module.is_null() {
		let error = CStr::from_ptr(dlerror()).to_string_lossy();
		panic!("{name}: {error}");
	}
	module
}

pub unsafe fn library_symbol(module: *mut c_void, symbol: &str) -> *mut c_void {
	let symbol = CString::new(symbol).unwrap();
	dlsym(module, symbol.as_ptr())
}

pub fn log(msg: &str) {
	println!("{msg}");
}

pub fn exit(code: i32) -> ! {
	std::process::exit(code)
}

/// Linux needs no setup beyond the exported libc interposers below.
pub unsafe fn init() {}

pub fn network_available() -> bool {
	(unsafe { CONFIG.local_ip.is_some() }) || local_ip_address::local_ip().is_ok()
}

pub unsafe fn load_plugins(version: &GameVersion) {
	for plugin in glob::glob("plugins/*.so").unwrap() {
		let plugin_name = plugin.unwrap().to_string_lossy().to_string();
		let plugin = CString::new(plugin_name.clone()).unwrap();
		let plugin = dlopen(plugin.as_ptr(), RTLD_LAZY);
		if plugin.is_null() {
			let error = dlerror();
			let error = CStr::from_ptr(error).to_string_lossy().to_string();
			panic!("{plugin_name} could not be loaded:  {error}");
		}
		let init = CString::new("init").unwrap();
		let init = dlsym(plugin, init.as_ptr());
		if init.is_null() {
			let error = dlerror();
			let error = CStr::from_ptr(error).to_string_lossy().to_string();
			panic!("init does not exist in {plugin_name}: {error}");
		}
		let init: fn(*const GameVersion) = transmute(init);
		init(version);
	}
}

#[no_mangle]
unsafe extern "C" fn sigaction() -> c_int {
	0
}

#[no_mangle]
unsafe extern "C" fn system(command: *const c_char) -> c_int {
	let cstr = CStr::from_ptr(command);
	let str = cstr.to_str().unwrap();

	if !CONFIG.block_sudo || str.starts_with("find") {
		let command = str.replace("/tmp/", "./tmp/");
		let command = CString::new(command).unwrap();

		let system = CString::new("system").unwrap();
		let system = dlsym(RTLD_NEXT, system.as_ptr());
		let system: extern "C" fn(*const c_char) -> c_int = transmute(system);

		let setenv = CString::new("setenv").unwrap();
		let setenv = dlsym(RTLD_DEFAULT, setenv.as_ptr());
		let setenv: extern "C" fn(*const c_char, *const c_char, c_int) -> c_int = transmute(setenv);

		let preload = CString::new("LD_PRELOAD").unwrap();
		let empty = CString::new("").unwrap();

		setenv(preload.as_ptr(), empty.as_ptr(), 1);
		system(command.as_ptr())
	} else {
		dbg!(str);
		0
	}
}

#[no_mangle]
unsafe extern "C" fn fopen(filename: *const c_char, mode: *const c_char) -> *const () {
	let filename = CStr::from_ptr(filename).to_str().unwrap();
	let filename = if filename.starts_with("/tmp") {
		CString::new(filename.replace("/tmp/", "./tmp/")).unwrap()
	} else {
		CString::new(filename).unwrap()
	};

	let fopen = CString::new("fopen").unwrap();
	let fopen = dlsym(RTLD_NEXT, fopen.as_ptr());
	let fopen: extern "C" fn(*const c_char, *const c_char) -> *const () = transmute(fopen);
	fopen(filename.as_ptr(), mode)
}

#[no_mangle]
unsafe extern "C" fn open(filename: *const c_char, flags: u32) -> *const () {
	let filename = CStr::from_ptr(filename).to_str().unwrap();
	let redirect = &CONFIG
		.file_redirect
		.as_ref()
		.map(|redirects| {
			redirects
				.iter()
				.filter(|redirect| redirect.from == filename)
				.next()
		})
		.flatten();

	let filename = if let Some(redirect) = redirect {
		CString::new(redirect.to.clone()).unwrap()
	} else if filename.starts_with("/tmp") {
		CString::new(filename.replace("/tmp/", "./tmp/")).unwrap()
	} else {
		CString::new(filename).unwrap()
	};

	let flags = if let Some(redirect) = redirect {
		if let Some(flags) = redirect.flags {
			flags
		} else {
			flags
		}
	} else {
		flags
	};

	let open = CString::new("open").unwrap();
	let open = dlsym(RTLD_NEXT, open.as_ptr());
	let open: extern "C" fn(*const c_char, u32) -> *const () = transmute(open);
	open(filename.as_ptr(), flags)
}

#[no_mangle]
unsafe extern "C" fn ioctl(fd: i32, op: u32, arg: *const c_void) -> i32 {
	let ioctl = CString::new("ioctl").unwrap();
	let ioctl = dlsym(RTLD_NEXT, ioctl.as_ptr());
	let ioctl: extern "C" fn(i32, u32, *const c_void) -> i32 = transmute(ioctl);
	let res = ioctl(fd, op, arg);
	if (op == 0x5463 || op == 0x5464) && CONFIG.ignore_custom_ioctls {
		0
	} else {
		res
	}
}

#[no_mangle]
unsafe extern "C" fn rename(old: *const c_char, new: *const c_char) -> c_int {
	let old = CStr::from_ptr(old).to_str().unwrap();
	let old = old.replace("/tmp/", "./tmp/");
	let old = CString::new(old).unwrap();

	let new = CStr::from_ptr(new).to_str().unwrap();
	let new = new.replace("/tmp/", "./tmp/");
	let new = CString::new(new).unwrap();

	let rename = CString::new("rename").unwrap();
	let rename = dlsym(RTLD_NEXT, rename.as_ptr());
	let rename: extern "C" fn(*const c_char, *const c_char) -> c_int = transmute(rename);
	rename(old.as_ptr(), new.as_ptr())
}

#[no_mangle]
unsafe extern "C" fn _ZNSt13basic_filebufIcSt11char_traitsIcEE4openEPKcSt13_Ios_Openmode(
	this: c_int,
	filename: *const c_char,
	mode: c_int,
) -> *const () {
	if let Ok(filename) = CStr::from_ptr(filename).to_str() {
		let filename = if filename.starts_with("/tmp") {
			CString::new(filename.replace("/tmp/", "./tmp/")).unwrap()
		} else {
			CString::new(filename).unwrap()
		};

		let open =
			CString::new("_ZNSt13basic_filebufIcSt11char_traitsIcEE4openEPKcSt13_Ios_Openmode")
				.unwrap();
		let open = dlsym(RTLD_NEXT, open.as_ptr());
		let open: extern "C" fn(c_int, *const c_char, c_int) -> *const () = transmute(open);
		open(this, filename.as_ptr(), mode)
	} else {
		let open =
			CString::new("_ZNSt13basic_filebufIcSt11char_traitsIcEE4openEPKcSt13_Ios_Openmode")
				.unwrap();
		let open = dlsym(RTLD_NEXT, open.as_ptr());
		let open: extern "C" fn(c_int, *const c_char, c_int) -> *const () = transmute(open);
		open(this, filename, mode)
	}
}

#[no_mangle]
unsafe extern "C" fn _ZNSt14basic_ifstreamIcSt11char_traitsIcEEC1EPKcSt13_Ios_Openmode(
	this: c_int,
	filename: *const c_char,
	mode: c_int,
) -> *const () {
	if let Ok(filename) = CStr::from_ptr(filename).to_str() {
		let filename = if filename.starts_with("/tmp") {
			CString::new(filename.replace("/tmp/", "./tmp/")).unwrap()
		} else if filename.starts_with("/proc/bus/usb/devices") {
			CString::new("./tmp/usb-devices").unwrap()
		} else {
			CString::new(filename).unwrap()
		};

		let open =
			CString::new("_ZNSt14basic_ifstreamIcSt11char_traitsIcEEC1EPKcSt13_Ios_Openmode")
				.unwrap();
		let open = dlsym(RTLD_NEXT, open.as_ptr());
		let open: extern "C" fn(c_int, *const c_char, c_int) -> *const () = transmute(open);
		open(this, filename.as_ptr(), mode)
	} else {
		let open =
			CString::new("_ZNSt14basic_ifstreamIcSt11char_traitsIcEEC1EPKcSt13_Ios_Openmode")
				.unwrap();
		let open = dlsym(RTLD_NEXT, open.as_ptr());
		let open: extern "C" fn(c_int, *const c_char, c_int) -> *const () = transmute(open);
		open(this, filename, mode)
	}
}
