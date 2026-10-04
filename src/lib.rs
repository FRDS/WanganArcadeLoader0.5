#![allow(static_mut_refs)]
use libc::*;
use poll::*;
use std::ffi::{CStr, CString};
use std::mem::transmute;
use std::str::FromStr;

pub mod adm;
pub mod al;
pub mod card;
pub mod hook;
pub mod jamma;
pub mod opengl;
pub mod platform;
pub mod poll;
pub mod res;
pub mod shader;

/// What to do about `data/shader`. `Auto` asks the driver whether it can run
/// the shipped programs and only replaces them if it can't; the other two are
/// escape hatches for a driver that misreports what it supports.
#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShaderMode {
	#[default]
	Auto,
	/// Always recompile to portable ARB.
	Portable,
	/// Never touch the files, whatever the driver reports.
	Original,
}

#[derive(serde::Deserialize)]
pub struct FileRedirect {
	from: String,
	to: String,
	flags: Option<u32>,
}

#[derive(serde::Deserialize)]
#[serde(default)]
pub struct Config {
	fullscreen: bool,
	input_emu: bool,
	card_emu: bool,
	block_sudo: bool,
	ignore_custom_ioctls: bool,
	dongle: String,
	local_ip: Option<String>,
	deadzone: f32,
	width: u32,
	height: u32,

	file_redirect: Option<Vec<FileRedirect>>,

	/// Frames per second cap, 0 for none. Linux usually caps with MangoHUD
	/// (see start.sh), so the default is 60 only on Windows.
	fps_limit: u32,

	/// Log controllers and every input to find binding names (see keyconfig.toml).
	input_log: bool,

	/// Log the game's Cg calls: the shader profile chosen for this GPU, and the
	/// compiler listing for any program that fails. Windows only — the engine's
	/// own shader reporting is compiled out of the release build it ships as.
	#[cfg_attr(unix, allow(dead_code))]
	cg_log: bool,

	/// Log every file the game opens. Windows only; failed opens are logged
	/// either way, since a path LINE can't reach fails silently otherwise.
	#[cfg_attr(unix, allow(dead_code))]
	file_log: bool,

	/// Ask for an OpenGL debug context and log the driver's own messages. The
	/// world is drawn with fixed-function and multitexture state, not Cg, so
	/// this is the only account of why it might come out wrong.
	gl_debug: bool,

	/// How to handle the dump's NVIDIA-only shaders. See `shader.rs`.
	shader_mode: ShaderMode,
}

impl Default for Config {
	fn default() -> Self {
		default_config()
	}
}

// Why cant this be a trait impl? Thanks rust
const fn default_config() -> Config {
	Config {
		fullscreen: false,
		input_emu: true,
		card_emu: true,
		block_sudo: true,
		ignore_custom_ioctls: true,
		dongle: String::new(),
		local_ip: None,
		deadzone: 0.01,
		width: 640,
		height: 480,
		file_redirect: None,
		fps_limit: if cfg!(windows) { 60 } else { 0 },
		input_log: false,
		cg_log: false,
		file_log: false,
		gl_debug: false,
		shader_mode: ShaderMode::Auto,
	}
}

pub struct KeyConfig {
	test: KeyBindings,
	service: KeyBindings,
	card_insert: KeyBindings,

	gear_next: KeyBindings,
	gear_previous: KeyBindings,
	gear_neutral: KeyBindings,
	gear_first: KeyBindings,
	gear_second: KeyBindings,
	gear_third: KeyBindings,
	gear_fourth: KeyBindings,
	gear_fifth: KeyBindings,
	gear_sixth: KeyBindings,
	gear_up: KeyBindings,
	gear_left: KeyBindings,
	gear_down: KeyBindings,
	gear_right: KeyBindings,

	perspective: KeyBindings,
	intrude: KeyBindings,
	gas: KeyBindings,
	brake: KeyBindings,
	wheel_left: KeyBindings,
	wheel_right: KeyBindings,
}

pub static mut CONFIG: Config = default_config();
pub static mut KEYCONFIG: Option<KeyConfig> = None;
pub static mut GAME_VERSION: GameVersion = default_gameversion();

pub extern "C" fn undachi() -> c_int {
	false as c_int
}

pub extern "C" fn adachi() -> c_int {
	true as c_int
}

static mut HASP_ID: i32 = 1;
unsafe extern "C" fn hasp_login(_: c_int, _: c_int, id: *mut c_int) -> c_int {
	id.write(HASP_ID);
	HASP_ID += 1;
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
	let dongle = CONFIG.dongle.as_bytes();
	let dongle = if dongle.len() < 12 {
		"285013501138".as_bytes()
	} else {
		dongle
	};
	data.as_mut_ptr()
		.offset(0xD00)
		.copy_from_nonoverlapping(dongle.as_ptr(), 12);
	let crc = data
		.iter()
		.take(0x0D)
		.fold(0, |acc: u8, data| acc.wrapping_add(*data));
	data[0x0D] = crc;
	data[0x0F] = std::ops::Not::not(crc);
	let crc = data
		.iter()
		.skip(0xD00)
		.take(0x3E)
		.fold(0, |acc: u8, data| acc.wrapping_add(*data));
	data[0xD3E] = crc;
	data[0xD3F] = std::ops::Not::not(crc);

	buffer.copy_from_nonoverlapping(data.as_ptr().offset(offset as isize), length as usize);
	0
}

static mut ORIGINAL_CL_MAIN: *const () = std::ptr::null();
unsafe extern "C" fn cl_main(log: *mut *mut ()) {
	platform::call_game::<_, ()>(ORIGINAL_CL_MAIN, (log,));
	log.write(hook::get_symbol("_ZSt4cout"));
}

unsafe extern "C" fn get_address(clnet: *mut *mut c_int) -> c_int {
	if let Some(local_ip) = &CONFIG.local_ip {
		let local_ip = std::net::Ipv4Addr::from_str(local_ip).unwrap();
		let ip = i32::from_be_bytes(local_ip.octets());
		let net = clnet.byte_offset(0x24).read();
		let net = if net.is_null() {
			clnet.byte_offset(0x1C).read()
		} else {
			net
		};
		net.byte_offset(0x04).write(ip);
		ip
	} else {
		let local_ip = local_ip_address::local_ip().unwrap();
		let local_ip = match local_ip {
			std::net::IpAddr::V4(addr) => addr,
			_ => unreachable!(),
		};
		let ip = i32::from_be_bytes(local_ip.octets());
		let net = clnet.byte_offset(0x24).read();
		let net = if net.is_null() {
			clnet.byte_offset(0x1C).read()
		} else {
			net
		};
		net.byte_offset(0x04).write(ip);
		net.byte_offset(0x04).write(ip);
		ip
	}
}

#[repr(C)]
pub struct RomInfo {
	name: [u8; 32],
	region: [u8; 32],
	release_type: [u8; 32],
	date: [u8; 32],
	time: [u8; 32],
	revision: c_int,
	revision_name: [u8; 32],
}

impl Into<GameVersion> for &RomInfo {
	fn into(self) -> GameVersion {
		let revision_name = CStr::from_bytes_until_nul(&self.revision_name);
		let Ok(revision_name) = revision_name else {
			return default_gameversion();
		};
		let Ok(revision_name) = revision_name.to_str() else {
			return default_gameversion();
		};

		let mut parts = revision_name.split('-');
		let Some(next) = parts.next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};
		let major = match next {
			"WM3100" => GameMajor::WM3,
			"W3X100" => GameMajor::W3X,
			"W3P100" => GameMajor::W3P,
			_ => {
				eprintln!("Unknown game revision: {revision_name}");
				return default_gameversion();
			}
		};

		let Some(next) = parts.next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};
		let region = match next {
			"1" => GameRegion::JP,
			"2" => GameRegion::EN2,
			"3" => GameRegion::EN3,
			"4" => GameRegion::EN4,
			_ => {
				eprintln!("Unknown game revision: {revision_name}");
				return default_gameversion();
			}
		};

		let Some(_) = parts.next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};
		let Some(_) = parts.next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};

		let Some(next) = parts.next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};

		let Some(minor) = next.chars().next() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};

		let minor = match minor {
			'A' => GameMinor::A,
			'B' => GameMinor::B,
			_ => {
				eprintln!("Unknown game revision: {revision_name}");
				return default_gameversion();
			}
		};

		let Ok(revision) = next.chars().skip(1).take(2).collect::<String>().parse() else {
			eprintln!("Unknown game revision: {revision_name}");
			return default_gameversion();
		};

		GameVersion {
			major,
			minor,
			region,
			revision,
		}
	}
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq)]
pub enum GameMajor {
	WM3 = 0,
	W3X = 1,
	W3P = 2,
	Unknown,
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq)]
pub enum GameMinor {
	A = 0,
	B = 1,
	Unknown,
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq)]
pub enum GameRegion {
	JP = 1,
	EN2 = 2,
	EN3 = 3,
	EN4 = 4,
	Unknown,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
pub struct GameVersion {
	major: GameMajor,
	minor: GameMinor,
	region: GameRegion,
	revision: u32,
}

const fn default_gameversion() -> GameVersion {
	GameVersion {
		major: GameMajor::Unknown,
		minor: GameMinor::Unknown,
		region: GameRegion::Unknown,
		revision: 0,
	}
}

// Test builds have no constructor calling init; keep it (and what it uses) "used".
#[cfg(test)]
const _: unsafe fn() = init;

/// Runs once before the game starts: from a constructor on Linux, and from
/// LINE's OnPreExecute("main") on Windows.
pub(crate) unsafe fn init() {
	if let Ok(toml) = std::fs::read_to_string("config.toml") {
		// Missing keys fall back to their defaults. A parse error keeps every
		// default, so say why instead of silently ignoring the whole file.
		match toml::from_str(&toml) {
			Ok(toml) => CONFIG = toml,
			Err(err) => eprintln!("config.toml not loaded, using defaults: {err}"),
		}
	}

	// Really what I should do is implement a custom serde::Deserialize for KeyBindings
	// but serdes documentation is really confusing when it comes to this
	#[derive(serde::Deserialize)]
	#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
	struct KeyConfigTemp {
		test: Vec<String>,
		service: Vec<String>,
		card_insert: Vec<String>,

		gear_next: Vec<String>,
		gear_previous: Vec<String>,
		gear_neutral: Vec<String>,
		gear_first: Vec<String>,
		gear_second: Vec<String>,
		gear_third: Vec<String>,
		gear_fourth: Vec<String>,
		gear_fifth: Vec<String>,
		gear_sixth: Vec<String>,
		gear_up: Vec<String>,
		gear_left: Vec<String>,
		gear_down: Vec<String>,
		gear_right: Vec<String>,

		perspective: Vec<String>,
		intrude: Vec<String>,
		gas: Vec<String>,
		brake: Vec<String>,
		wheel_left: Vec<String>,
		wheel_right: Vec<String>,
	}

	let toml = std::fs::read_to_string("keyconfig.toml").unwrap();
	let keyconfig: KeyConfigTemp = toml::from_str(&toml).unwrap();
	let keyconfig = KeyConfig {
		test: parse_keybinding(keyconfig.test),
		service: parse_keybinding(keyconfig.service),
		card_insert: parse_keybinding(keyconfig.card_insert),

		gear_next: parse_keybinding(keyconfig.gear_next),
		gear_previous: parse_keybinding(keyconfig.gear_previous),
		gear_neutral: parse_keybinding(keyconfig.gear_neutral),
		gear_first: parse_keybinding(keyconfig.gear_first),
		gear_second: parse_keybinding(keyconfig.gear_second),
		gear_third: parse_keybinding(keyconfig.gear_third),
		gear_fourth: parse_keybinding(keyconfig.gear_fourth),
		gear_fifth: parse_keybinding(keyconfig.gear_fifth),
		gear_sixth: parse_keybinding(keyconfig.gear_sixth),
		gear_up: parse_keybinding(keyconfig.gear_up),
		gear_left: parse_keybinding(keyconfig.gear_left),
		gear_down: parse_keybinding(keyconfig.gear_down),
		gear_right: parse_keybinding(keyconfig.gear_right),

		perspective: parse_keybinding(keyconfig.perspective),
		intrude: parse_keybinding(keyconfig.intrude),
		gas: parse_keybinding(keyconfig.gas),
		brake: parse_keybinding(keyconfig.brake),
		wheel_left: parse_keybinding(keyconfig.wheel_left),
		wheel_right: parse_keybinding(keyconfig.wheel_right),
	};
	KEYCONFIG = Some(keyconfig);

	platform::init();
	// Before the engine reads them, so the fixed files are the ones it loads.
	shader::ensure_portable(CONFIG.shader_mode);

	hook::hook_symbol("hasp_cleanup", undachi as *const ());
	hook::hook_symbol("hasp_decrypt", undachi as *const ());
	hook::hook_symbol("hasp_encrypt", undachi as *const ());
	hook::hook_symbol("hasp_free", undachi as *const ());
	hook::hook_symbol("hasp_get_rtc", undachi as *const ());
	hook::hook_symbol("hasp_get_sessioninfo", undachi as *const ());
	hook::hook_symbol("hasp_get_size", hasp_size as *const ());
	hook::hook_symbol("hasp_legacy_set_rtc", undachi as *const ());
	hook::hook_symbol("hasp_login", hasp_login as *const ());
	hook::hook_symbol("hasp_logout", undachi as *const ());
	hook::hook_symbol("hasp_read", hasp_read as *const ());
	hook::hook_symbol("hasp_write", undachi as *const ());
	hook::hook_symbol("hasp_get_rtc", undachi as *const ());
	hook::hook_symbol("hasp_hasptime_to_datetime", undachi as *const ());
	hook::hook_symbol("_ZNK6clHasp7isAvailEv", adachi as *const ());

	if platform::network_available() {
		hook::hook_symbol("_ZNK5clNet10getAddressEv", get_address as *const ());
	} else {
		hook::hook_symbol("_ZN18clSeqBootNetThread3runEPv", adachi as *const ());
	}

	ORIGINAL_CL_MAIN = hook::hook_symbol("_ZN6clMainC1Ev", cl_main as *const ());

	adm::init();
	al::load_al_funcs();

	if CONFIG.input_emu {
		jamma::init();
	}
	if CONFIG.card_emu {
		card::init();
	}
	if CONFIG.width != 640 || CONFIG.height != 480 {
		res::init();
	}

	let rom_info = hook::get_symbol("gRomInfo") as *const RomInfo;
	let rom_info = rom_info.as_ref().unwrap();
	let version = rom_info.into();
	GAME_VERSION = version;
	platform::load_plugins(&version);
}
