//! Boot hooks needed to get the game from main's entry to admCreateWindowi.
//! Ported from the Linux loader (src/lib.rs, src/jamma.rs) and from
//! line-wal0.5's lib.cpp / jamma.cpp.

use crate::{adm, al, card, line};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
#[serde(default)]
pub struct Config {
	pub card_emu: bool,
	pub dongle: String,
}

impl Default for Config {
	fn default() -> Self {
		Config {
			card_emu: true,
			dongle: String::new(),
		}
	}
}

static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn config() -> &'static Config {
	CONFIG.get_or_init(Config::default)
}

fn load_config() -> Config {
	match std::fs::read_to_string("config.toml") {
		Ok(text) => match toml::from_str(&text) {
			Ok(config) => config,
			Err(err) => {
				log!("config.toml: parse error, using defaults: {err}");
				Config::default()
			}
		},
		Err(err) => {
			log!("config.toml: not read ({err}), using defaults");
			Config::default()
		}
	}
}

extern "C" {
	fn wal_call1(f: *mut c_void, a: *mut c_void);
	fn wal_align_shim_is_aligned() -> c_int;
}

pub fn align_shim_is_aligned() -> bool {
	unsafe { wal_align_shim_is_aligned() != 0 }
}

pub extern "C" fn undachi() -> c_int {
	0
}

pub extern "C" fn adachi() -> c_int {
	1
}

// Milestones reported in the final summary.
pub static HASP_LOGIN_CALLED: AtomicBool = AtomicBool::new(false);
pub static CL_MAIN_RETURNED: AtomicBool = AtomicBool::new(false);
pub static SYSTEM_CALLS: AtomicU32 = AtomicU32::new(0);

static HASP_ID: AtomicU32 = AtomicU32::new(1);
unsafe extern "C" fn hasp_login(_: c_int, _: c_int, id: *mut c_int) -> c_int {
	if !HASP_LOGIN_CALLED.swap(true, Ordering::Relaxed) {
		log!("hasp_login called");
	}
	id.write(HASP_ID.fetch_add(1, Ordering::Relaxed) as c_int);
	0
}

unsafe extern "C" fn hasp_size(_: c_int, _: c_int, size: *mut c_int) -> c_int {
	size.write(0xD40);
	0
}

unsafe extern "C" fn hasp_read(
	_: c_int,
	_: c_int,
	offset: c_int,
	length: c_int,
	buffer: *mut u8,
) -> c_int {
	let mut data = [0u8; 0xD40];
	let dongle = config().dongle.as_bytes();
	let dongle = if dongle.len() < 12 {
		"285013501138".as_bytes()
	} else {
		dongle
	};
	data[0xD00..0xD00 + 12].copy_from_slice(&dongle[..12]);
	let mut crc: u8 = 0;
	for byte in &data[..=0x0D] {
		crc = crc.wrapping_add(*byte);
	}
	data[0x0D] = crc;
	data[0x0F] = !crc;
	crc = 0;
	for byte in &data[0xD00..=0xD00 + 62] {
		crc = crc.wrapping_add(*byte);
	}
	data[0xD3E] = crc;
	data[0xD3F] = !crc;

	let (offset, length) = (offset as usize, length as usize);
	if offset + length > data.len() {
		log!("hasp_read out of range: offset {offset:#x} length {length:#x}");
		return 1;
	}
	buffer.copy_from_nonoverlapping(data.as_ptr().add(offset), length);
	0
}

// clMain's constructor. Calling the original is the test's only call from
// Rust back into game code before the window, so it goes through the shim.
static ORIGINAL_CL_MAIN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
unsafe extern "C" fn cl_main(this: *mut *mut c_void) {
	log!("cl_main called, calling original through the align shim");
	wal_call1(ORIGINAL_CL_MAIN.load(Ordering::Relaxed), this as *mut c_void);
	CL_MAIN_RETURNED.store(true, Ordering::Relaxed);
	log!("cl_main original returned");
	this.write(line::dl_sym("_ZSt4cout"));
}

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
			log!("system: can't open {dir}: {err}");
			Vec::new()
		}
	};
	names.sort();
	let list: String = names.iter().map(|name| format!("{dir}/{name}\n")).collect();
	if let Err(err) = std::fs::write("tmp/find.txt", list) {
		log!("system: can't write tmp/find.txt: {err}");
	}
}

fn copy_targets() {
	let entries = match std::fs::read_dir("data/target") {
		Ok(entries) => entries,
		Err(err) => {
			log!("system: can't open data/target: {err}");
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
			log!("system: can't copy {name} to {dest}: {err}");
		}
	}
}

// Replaces LINE's system() stub, which only prints the command. There is no
// shell, so the commands the game relies on are emulated here.
unsafe extern "C" fn system(command: *const c_char) -> c_int {
	SYSTEM_CALLS.fetch_add(1, Ordering::Relaxed);
	if command.is_null() {
		return 0;
	}
	let command = CStr::from_ptr(command).to_string_lossy();
	log!("system({command})");

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
		log!("system: unhandled command, ignored");
	}
	0
}

pub unsafe fn init() {
	_ = CONFIG.set(load_config());

	for symbol in [
		"hasp_cleanup",
		"hasp_decrypt",
		"hasp_encrypt",
		"hasp_free",
		"hasp_get_rtc",
		"hasp_get_sessioninfo",
		"hasp_legacy_set_rtc",
		"hasp_logout",
		"hasp_write",
		"hasp_hasptime_to_datetime",
	] {
		line::hook_symbol(symbol, undachi as *const c_void);
	}
	line::hook_symbol("hasp_get_size", hasp_size as *const c_void);
	line::hook_symbol("hasp_login", hasp_login as *const c_void);
	line::hook_symbol("hasp_read", hasp_read as *const c_void);
	line::hook_symbol("_ZNK6clHasp7isAvailEv", adachi as *const c_void);

	if let Some(original) = line::hook_symbol("_ZN6clMainC1Ev", cl_main as *const c_void) {
		ORIGINAL_CL_MAIN.store(original, Ordering::Relaxed);
	}

	// Skip network setup.
	line::hook_symbol("_ZN18clSeqBootNetThread3runEPv", adachi as *const c_void);
	line::hook_symbol("_ZN5clNet19setInterfaceAddressEv", adachi as *const c_void);

	match line::signature("2f 70 72 6f 63 2f 62 75 73") {
		Some(address) => {
			line::patch_bytes(address, b"tmp/usb-devices\0");
			log!("patched /proc/bus/usb/devices -> tmp/usb-devices at {address:p}");
		}
		None => log!("warning: /proc/bus/usb/devices string not found"),
	}

	let system_stub = line::resolve_stub("system");
	if line::hook(system_stub, system as *const c_void).is_some() {
		log!("hooked system() stub at {system_stub:p}");
	} else {
		log!("failed to hook system() stub at {system_stub:p}");
	}

	// Jamma I/O board: stub out the hardware, no input handling in this test.
	for symbol in [
		"_ZN10clSystemN24initEb",
		"_ZN10clSystemN212initSystemN2Ev",
		"_ZN18clInputDeviceJamma8checkUseEv",
		"_ZN18clInputDeviceJamma12handleEventsEv",
		"_ZN16clInputDevicePad12handleEventsEv",
		"_ZN16clInputDevicePad13joyButtonDownEPN3Gap7Display12igControllerENS2_7BUTTONSE",
		"_ZN16clInputDevicePad17joyButtonPressureEPN3Gap7Display12igControllerENS2_7BUTTONSEf",
		"_ZN16clInputDevicePad11joyButtonUpEPN3Gap7Display12igControllerENS2_7BUTTONSE",
		"_ZN16clInputDevicePad8joyStickEPN3Gap7Display12igControllerEtff",
		"n2JvioTxVsync",
		"n2JvioAckTxVsync",
	] {
		line::hook_symbol(symbol, adachi as *const c_void);
	}

	if config().card_emu {
		card::init();
	}
	al::init();
	adm::init();

	log!(
		"boot: {} hooks, {} missing, {} failed",
		line::HOOKED.load(Ordering::Relaxed),
		line::MISSING.load(Ordering::Relaxed),
		line::FAILED.load(Ordering::Relaxed)
	);
}
