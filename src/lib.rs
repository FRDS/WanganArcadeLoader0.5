#![allow(static_mut_refs)]
use libc::*;
use poll::*;
use std::ffi::{CStr, CString};
use std::mem::transmute;
use std::str::FromStr;

pub mod adm;
pub mod al;
pub mod card;
pub mod elfsym;
pub mod hook;
pub mod hud;
pub mod jamma;
pub mod opengl;
pub mod platform;
pub mod poll;
pub mod report;
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

	/// Change the display mode to `width` x `height` instead of filling the
	/// monitor's current mode. Exact when the monitor offers that mode, so
	/// there is no letterbox and no upscale; GLFW silently falls back to the
	/// closest mode when it does not, which `adm.rs` logs. Off by default:
	/// borderless is the proven path, and a mode change disturbs the desktop.
	fullscreen_exclusive: bool,
	input_emu: bool,
	card_emu: bool,
	block_sudo: bool,
	ignore_custom_ioctls: bool,

	/// Ignore SIGALRM, which the game arms a 50 Hz timer for and never handles.
	/// Left to itself that signal ends the process at a random moment with no
	/// log, no stackdump and no error -- the "it closed by itself" ending.
	/// Windows only: on the arcade's own Linux something absorbed it.
	#[cfg_attr(unix, allow(dead_code))]
	ignore_alarm: bool,
	dongle: String,
	local_ip: Option<String>,
	deadzone: f32,
	width: u32,
	height: u32,

	/// Correct the engine's 2D projection when the render is not 4:3. The dump
	/// lays its 2D layer out in 640x480 units and the engine maps that onto
	/// whatever the viewport is, so a 16:9 render stretches the whole layer by
	/// a third; this widens the projection instead, leaving the layer centred
	/// and showing world where the stretch used to be. A no-op at 4:3. Turn it
	/// off to get the old behaviour without giving up the resolution.
	widescreen_hud: bool,

	file_redirect: Option<Vec<FileRedirect>>,

	/// Wait for the display's refresh before swapping. Right on any display
	/// that holds 60; turn it off on one that cannot, since the frame rate is
	/// the game's speed, and let fps_limit do the pacing instead.
	vsync: bool,

	/// Frames per second cap, 0 for none. **This sets the game's speed**: the
	/// engine takes one simulation step per frame, so 60 is not a preference
	/// but the rate the arcade hardware ran at. Linux usually caps with
	/// MangoHUD (see start.sh), so the default is 60 only on Windows.
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

	/// Log every card-printer command, its payload size and the status the
	/// printer answers with. Commands the emulation does not implement, and
	/// writes it refuses, are reported either way.
	card_log: bool,

	/// Log the geometry the engine asks for: the screen size it reads out of
	/// Lua, the viewports, the perspective FOV and every orthographic box, one
	/// line per distinct set of arguments. Most of it needs a custom
	/// resolution, since that is when `res.rs` installs its hooks at all; the
	/// fixed-function `glOrtho` is watched at any resolution.
	res_log: bool,

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
		fullscreen_exclusive: false,
		input_emu: true,
		card_emu: true,
		block_sudo: true,
		ignore_custom_ioctls: true,
		ignore_alarm: true,
		dongle: String::new(),
		local_ip: None,
		deadzone: 0.01,
		width: 640,
		height: 480,
		widescreen_hud: true,
		file_redirect: None,
		vsync: true,
		fps_limit: if cfg!(windows) { 60 } else { 0 },
		input_log: false,
		cg_log: false,
		file_log: false,
		card_log: false,
		res_log: false,
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

/// The address to tell the game it has: `local_ip` when it is set, otherwise
/// this machine's. `None` if neither can be had, which is only reachable if
/// the answer changed since `network_available()` agreed there was one.
unsafe fn own_address() -> Option<i32> {
	let address = match &CONFIG.local_ip {
		Some(text) => match std::net::Ipv4Addr::from_str(text) {
			Ok(address) => address,
			Err(err) => {
				net_log(format!(
					"net: local_ip is not an IPv4 address ({err}): {text}"
				));
				return None;
			}
		},
		None => match local_ip_address::local_ip() {
			Ok(std::net::IpAddr::V4(address)) => address,
			Ok(std::net::IpAddr::V6(address)) => {
				net_log(format!(
					"net: this machine's address is IPv6 ({address}), and the game only \
					 understands IPv4 -- set local_ip"
				));
				return None;
			}
			Err(err) => {
				net_log(format!("net: cannot find this machine's address: {err}"));
				return None;
			}
		},
	};
	Some(i32::from_be_bytes(address.octets()))
}

/// `clNet::getAddress`, where the game learns its own address.
///
/// The net struct it reads from sits at a different offset per build --
/// `+0x1C` on base WMMT3, `+0x24` on 3DX and 3DX+ -- so the revision decides
/// which, with the other tried as a fallback for a build nobody has looked at.
/// Reporting nothing used to be a panic: an unparseable `local_ip`, or no
/// network at all, took the process.
unsafe extern "C" fn get_address(clnet: *mut *mut c_int) -> c_int {
	let Some(ip) = own_address() else {
		return 0;
	};
	let (offset, fallback) = if GAME_VERSION.major == GameMajor::WM3 {
		(0x1C, 0x24)
	} else {
		(0x24, 0x1C)
	};
	let mut net = clnet.byte_offset(offset).read();
	if net.is_null() {
		net_log(format!(
			"net: clNet+{offset:#x} holds no net struct, trying +{fallback:#x}"
		));
		net = clnet.byte_offset(fallback).read();
	}
	if net.is_null() {
		net_log(String::from(
			"net: clNet has no net struct at either offset, so its address is not ours to set",
		));
		return ip;
	}
	net.byte_offset(0x04).write(ip);
	ip
}

/// Says something once. `getAddress` is a getter the game may poll, so none of
/// what it reports may repeat per call.
fn net_log(message: String) {
	static REPORTED: report::Reported = report::Reported::new();
	REPORTED.say(message);
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

impl From<&RomInfo> for GameVersion {
	fn from(rom_info: &RomInfo) -> Self {
		let name = CStr::from_bytes_until_nul(&rom_info.revision_name)
			.ok()
			.and_then(|name| name.to_str().ok());
		let Some(name) = name else {
			platform::log("gRomInfo's revision name is not text, so the revision stays unknown");
			return default_gameversion();
		};
		match parse_revision(name) {
			Some(version) => {
				platform::log(&format!("game revision {name}"));
				version
			}
			None => {
				platform::log(&format!("unknown game revision: {name}"));
				default_gameversion()
			}
		}
	}
}

/// `W3P100-2-NA-DAT0-B02` -- the Export 3DX+ dump, read off it at boot --
/// into its parts. `None` for
/// anything that is not that shape, which is the caller's cue to say so: every
/// version-gated offset in the loader reads what this returns, and an
/// unrecognised name silently takes the non-WMMT3 branch of each.
fn parse_revision(name: &str) -> Option<GameVersion> {
	let mut parts = name.split('-');
	let major = match parts.next()? {
		"WM3100" => GameMajor::WM3,
		"W3X100" => GameMajor::W3X,
		"W3P100" => GameMajor::W3P,
		_ => return None,
	};
	let region = match parts.next()? {
		"1" => GameRegion::JP,
		"2" => GameRegion::EN2,
		"3" => GameRegion::EN3,
		"4" => GameRegion::EN4,
		_ => return None,
	};
	// Two fields nothing is known about, then the revision itself.
	parts.next()?;
	parts.next()?;
	let revision_name = parts.next()?;
	let minor = match revision_name.chars().next()? {
		'A' => GameMinor::A,
		'B' => GameMinor::B,
		_ => return None,
	};
	let revision = revision_name
		.chars()
		.skip(1)
		.take(2)
		.collect::<String>()
		.parse()
		.ok()?;
	Some(GameVersion {
		major,
		minor,
		region,
		revision,
	})
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GameMajor {
	WM3 = 0,
	W3X = 1,
	W3P = 2,
	Unknown,
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GameMinor {
	A = 0,
	B = 1,
	Unknown,
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GameRegion {
	JP = 1,
	EN2 = 2,
	EN3 = 3,
	EN4 = 4,
	Unknown,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
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
	//
	// `default` on each field so one missing line unbinds one action instead of
	// failing the whole file -- which used to abort the process, with no window
	// and nothing on screen to say why.
	#[derive(Default, serde::Deserialize)]
	#[serde(default, rename_all = "SCREAMING_SNAKE_CASE")]
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

	// The shipped file, as the fallback for a keyconfig.toml that is missing or
	// unreadable: the documented default bindings are better than none, and
	// `shipped_keyconfig_parses` already guarantees every name in it parses.
	const SHIPPED_KEYCONFIG: &str = include_str!("../dist/keyconfig.toml");
	let shipped = || toml::from_str::<KeyConfigTemp>(SHIPPED_KEYCONFIG).unwrap_or_default();
	let keyconfig = match std::fs::read_to_string("keyconfig.toml") {
		Ok(toml) => match toml::from_str(&toml) {
			Ok(keyconfig) => keyconfig,
			Err(err) => {
				platform::log(&format!(
					"input: keyconfig.toml not loaded, using the shipped bindings: {err}"
				));
				shipped()
			}
		},
		Err(err) => {
			platform::log(&format!(
				"input: cannot read keyconfig.toml, using the shipped bindings: {err}"
			));
			shipped()
		}
	};

	// An action nothing parsed for is an action the player cannot reach, and a
	// mistyped key name reads as an absent one, so say which.
	let mut unbound: Vec<&str> = Vec::new();
	let mut bind = |name: &'static str, keys: Vec<String>| {
		let parsed = parse_keybinding(keys);
		if parsed.is_empty() {
			unbound.push(name);
		}
		parsed
	};
	let keyconfig = KeyConfig {
		test: bind("TEST", keyconfig.test),
		service: bind("SERVICE", keyconfig.service),
		card_insert: bind("CARD_INSERT", keyconfig.card_insert),

		gear_next: bind("GEAR_NEXT", keyconfig.gear_next),
		gear_previous: bind("GEAR_PREVIOUS", keyconfig.gear_previous),
		gear_neutral: bind("GEAR_NEUTRAL", keyconfig.gear_neutral),
		gear_first: bind("GEAR_FIRST", keyconfig.gear_first),
		gear_second: bind("GEAR_SECOND", keyconfig.gear_second),
		gear_third: bind("GEAR_THIRD", keyconfig.gear_third),
		gear_fourth: bind("GEAR_FOURTH", keyconfig.gear_fourth),
		gear_fifth: bind("GEAR_FIFTH", keyconfig.gear_fifth),
		gear_sixth: bind("GEAR_SIXTH", keyconfig.gear_sixth),
		gear_up: bind("GEAR_UP", keyconfig.gear_up),
		gear_left: bind("GEAR_LEFT", keyconfig.gear_left),
		gear_down: bind("GEAR_DOWN", keyconfig.gear_down),
		gear_right: bind("GEAR_RIGHT", keyconfig.gear_right),

		perspective: bind("PERSPECTIVE", keyconfig.perspective),
		intrude: bind("INTRUDE", keyconfig.intrude),
		gas: bind("GAS", keyconfig.gas),
		brake: bind("BRAKE", keyconfig.brake),
		wheel_left: bind("WHEEL_LEFT", keyconfig.wheel_left),
		wheel_right: bind("WHEEL_RIGHT", keyconfig.wheel_right),
	};
	if !unbound.is_empty() {
		platform::log(&format!("input: nothing bound to {}", unbound.join(", ")));
	}
	KEYCONFIG = Some(keyconfig);

	// Before anything is installed, because a hook that gates on the revision
	// has to be able to read it while it is being installed. This used to run
	// last, and worked only because its one reader looks at it per frame.
	let rom_info = (hook::get_symbol("gRomInfo") as *const RomInfo).as_ref();
	if rom_info.is_none() {
		platform::log("gRomInfo is missing, so the game revision stays unknown");
	}
	GAME_VERSION = rom_info.map_or_else(default_gameversion, GameVersion::from);

	platform::init();
	// Read the game's symbols up front: the crash reporter needs them to name
	// a function, and an exception handler is no place to open a file.
	elfsym::load();
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
	// Measurement only, and only when asked for; see the module comment.
	hud::init();

	platform::load_plugins(&GAME_VERSION);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The dump every hardware run in `docs/ROADMAP.md` was made on, read off
	/// it at boot. The two middle fields are not the `0-0` an earlier note
	/// guessed at, which is the reason this test exists at all.
	#[test]
	fn reads_the_export_3dxp_revision() {
		let version = parse_revision("W3P100-2-NA-DAT0-B02").expect("the Export 3DX+ name");
		assert!(version.major == GameMajor::W3P);
		assert!(version.region == GameRegion::EN2);
		assert!(version.minor == GameMinor::B);
		assert_eq!(version.revision, 2);
	}

	#[test]
	fn reads_the_other_two_majors() {
		// WM3100 is the one `adm.rs` and `get_address` branch on, so it is the
		// one a parse failure would quietly misidentify.
		assert!(parse_revision("WM3100-1-0-0-B12").unwrap().major == GameMajor::WM3);
		assert!(parse_revision("WM3100-1-0-0-B12").unwrap().minor == GameMinor::B);
		assert_eq!(parse_revision("WM3100-1-0-0-B12").unwrap().revision, 12);
		assert!(parse_revision("W3X100-4-0-0-A03").unwrap().major == GameMajor::W3X);
		assert!(parse_revision("W3X100-4-0-0-A03").unwrap().region == GameRegion::EN4);
	}

	#[test]
	fn refuses_a_name_it_does_not_understand() {
		for name in [
			"",
			"W3P100",
			"W3P100-2",
			"W3P100-2-0-0",     // nothing where the revision goes
			"W3P100-9-0-0-A01", // no such region
			"W3P100-2-0-0-C01", // no such minor
			"W3P100-2-0-0-A",   // no number after the letter
			"W3P100-2-0-0-Axx", // and not a number either
			"WM4100-2-0-0-A01", // a game this is not
		] {
			assert!(parse_revision(name).is_none(), "accepted {name:?}");
		}
	}

	/// A `RomInfo` whose name is not NUL-terminated, or not UTF-8, must come
	/// back unknown rather than reading past the field.
	#[test]
	fn survives_a_rom_info_that_is_not_text() {
		fn named(bytes: &[u8]) -> RomInfo {
			let mut revision_name = [0_u8; 32];
			revision_name[..bytes.len()].copy_from_slice(bytes);
			RomInfo {
				name: [0; 32],
				region: [0; 32],
				release_type: [0; 32],
				date: [0; 32],
				time: [0; 32],
				revision: 0,
				revision_name,
			}
		}

		// The whole field used, so there is no terminator to find.
		assert!(GameVersion::from(&named(&[b'A'; 32])).major == GameMajor::Unknown);
		// Terminated, with rubbish after it that is none of our business.
		let mut padded = *b"W3P100-2-NA-DAT0-B02\0\xff\xfe";
		assert!(GameVersion::from(&named(&mut padded)).major == GameMajor::W3P);
		// Terminated, but not text.
		assert!(GameVersion::from(&named(b"\xff\xfe\0")).major == GameMajor::Unknown);
	}
}
