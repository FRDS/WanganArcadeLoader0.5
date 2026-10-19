//! Custom race BGM, ported from `vixen256/audio_3dxp` (GPLv3).
//!
//! A pack is a folder under `plugins/bgms/` holding `mod.toml` and `mod.png`,
//! the layout `audio_3dxp` defined, so packs made for the plugin work here
//! unchanged. Each pack becomes another set in the BGM select screen, beside
//! the three the game ships.
//!
//! **The game's own catalogue is read out of the binary rather than
//! hardcoded.** `audio_3dxp` carries three tables of paths and song titles;
//! both are in the dump, so the native sets here are whatever this build
//! actually has:
//!
//! - `BGM_TABLE` is `nsAudio`'s table, 111 entries of 12 bytes indexed by
//!   `emBgmType`: the WAV path, the relative volume, and the title as a
//!   `wchar_t` string already in the game's `N Name` display form.
//! - `MENU_TABLE` is the select screen's `int[3][25]`, read as
//!   `[set * 25 + index]`, each row starting with the RANDOM entry and ending
//!   with `-1`.
//!
//! **Every song plays through entry 0 of `BGM_TABLE`.** `clAudioStream::play`
//! loads `ebx` with `table[id].path`, reads the header through
//! `clWaveFile::loadFile`, and then `fopen`s `ebx` again to stream the body,
//! so substituting the path *inside* `loadFile` -- which is what `audio_3dxp`
//! does -- gets the header from one file and the audio from another, and needs
//! assembly to force `ebx` afterwards. Writing the path into the table before
//! the call instead feeds both reads from the same place. Entry 0 is left
//! holding whatever is playing, which is also what makes `getMusicTitle`
//! report the right name with no hook on it: the game asks for the title of
//! the id it was given, and that id is 0.
//!
//! Credit to vixen256 for the mechanism, the id encoding and the card
//! handling; this is a port of their work.

use crate::platform::call_game;
use crate::*;

/// `nsAudio`'s BGM table: `{ const char *path, float volume, const wchar_t
/// *title }`, indexed by `emBgmType`.
const BGM_TABLE: usize = 0x0932_1260;
const BGM_ENTRY: usize = 12;
const BGM_ENTRIES: usize = 111;

/// The select screen's id table, read as `[set * MENU_SLOTS + index]`.
const MENU_TABLE: usize = 0x093E_3560;
const MENU_SETS: usize = 3;
const MENU_SLOTS: usize = 25;

/// The entry every custom song is played through. Its path is
/// `data/sound/bgm/maxi1/wm2_05.wav`, which is also the file `audio_3dxp`
/// keyed its substitution on.
const SCRATCH: i32 = 0;

/// `clSeqCardDisp9::run`, where the chosen set is clamped to at most 2 on its
/// way into the card: `cmp al, 3; cmovge eax, edx`. A custom set index only
/// survives the trip if the `cmovge` is gone.
const CLAMP: usize = 0x087E_F4FC;
const CLAMP_BYTES: [u8; 3] = [0x0f, 0x4d, 0xc2];

/// Where the card keeps the chosen BGM set, in `clV386CardData`'s buffer.
const CARD_BGM: usize = 0x16D;

/// The first custom id. Below this an id is the game's own `emBgmType` and is
/// forwarded untouched.
const ENCODED: i32 = 1 << 8;

/// The RANDOM entry the game puts first in every set.
const RANDOM_ID: i32 = 0x6E;

struct Song {
	/// NUL-terminated, kept alive because its pointer goes into the table.
	path: std::ffi::CString,
	/// The title as the game stores them: NUL-terminated 4-byte `wchar_t`.
	title: Vec<u32>,
	/// The title as text, for matching `stories` entries and for the log.
	name: String,
}

struct Set {
	name: String,
	image: Option<std::ffi::CString>,
	songs: Vec<Song>,
	/// Song names to play in each story episode, by episode number.
	stories: Option<Vec<String>>,
}

static mut SETS: Vec<Set> = Vec::new();
/// The set whose icon was asked for last, so `igImage::load` knows which
/// pack's image to answer the maxi3 logo with.
static mut SHOWN_SET: i32 = 0;
/// What the card really held, so saving it back never records a set the
/// arcade would not understand.
static mut CARD_SET: u8 = 0;
static REPORTED: report::Reported = report::Reported::new();

// --------------------------------------------------------------- the id codec
//
// `audio_3dxp`'s encoding, kept exactly so a card written by one is read the
// same way by the other: the set in the high bits, a 1-based song index in
// the low byte, index 0 meaning RANDOM.

fn encode(set: i32, index: i32) -> i32 {
	(set + 1) << 8 | index
}

fn set_of(id: i32) -> i32 {
	if id == -1 {
		0
	} else {
		(id >> 8) - 1
	}
}

fn index_of(id: i32) -> i32 {
	if id == -1 || id == RANDOM_ID {
		0
	} else {
		id & (ENCODED - 1)
	}
}

/// The next index in `set`, wrapping back to RANDOM past the last song.
fn next_index(songs: usize, index: i32) -> i32 {
	if index as usize >= songs {
		0
	} else {
		index + 1
	}
}

// ------------------------------------------------------------------- the dump
//
// Reading the two tables. Bounded by their measured lengths rather than by a
// terminator, because an id that is out of range is a bug to report, not a
// pointer to follow.

unsafe fn entry(id: i32) -> Option<*mut u8> {
	if id < 0 || id as usize >= BGM_ENTRIES {
		return None;
	}
	Some((BGM_TABLE + id as usize * BGM_ENTRY) as *mut u8)
}

unsafe fn native_path(id: i32) -> Option<String> {
	let pointer = entry(id)?.cast::<*const c_char>().read();
	if pointer.is_null() {
		return None;
	}
	Some(CStr::from_ptr(pointer).to_string_lossy().into_owned())
}

/// The title as the game stores it, and as text.
unsafe fn native_title(id: i32) -> Option<(Vec<u32>, String)> {
	let pointer = entry(id)?.add(8).cast::<*const u32>().read();
	if pointer.is_null() {
		return None;
	}
	let mut wide = Vec::new();
	loop {
		let unit = pointer.add(wide.len()).read();
		if unit == 0 {
			break;
		}
		wide.push(unit);
		if wide.len() > 256 {
			break;
		}
	}
	let text: String = wide
		.iter()
		.filter_map(|&unit| char::from_u32(unit))
		.collect();
	wide.push(0);
	Some((wide, text))
}

/// The ids in one native set, in menu order, without the leading RANDOM entry
/// and stopping at the `-1` that ends the row.
unsafe fn native_set(set: usize) -> Vec<i32> {
	let mut ids = Vec::new();
	for slot in 0..MENU_SLOTS {
		let id = ((MENU_TABLE + (set * MENU_SLOTS + slot) * 4) as *const i32).read();
		if id == -1 {
			break;
		}
		if id == RANDOM_ID {
			continue;
		}
		ids.push(id);
	}
	ids
}

/// The three sets the game ships, read from its own tables so the names, the
/// order and the paths are this build's rather than a remembered copy.
unsafe fn native_sets() -> Vec<Set> {
	let mut sets = Vec::new();
	for index in 0..MENU_SETS {
		let mut songs = Vec::new();
		for id in native_set(index) {
			let Some(path) = native_path(id) else {
				platform::log(&format!("bgm: native set {index} has no path for id {id}"));
				continue;
			};
			let Some((title, name)) = native_title(id) else {
				platform::log(&format!("bgm: native set {index} has no title for id {id}"));
				continue;
			};
			songs.push(Song {
				path: std::ffi::CString::new(path).unwrap_or_default(),
				title,
				name,
			});
		}
		sets.push(Set {
			// What the select screen calls them, and what a card written by
			// `audio_3dxp` names in `audio.toml`.
			name: ["3", "2", "R"][index].to_string(),
			image: None,
			songs,
			stories: None,
		});
	}
	sets
}

// -------------------------------------------------------------------- packs

/// One pack's `mod.toml`.
#[derive(serde::Deserialize)]
struct PackFile {
	song: Vec<PackSong>,
	stories: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct PackSong {
	name: String,
	file: String,
}

/// A title in the form the game's own entries use, so a custom song looks the
/// same in the list as a shipped one.
fn wide_title(index: usize, name: &str) -> (Vec<u32>, String) {
	let text = format!("{} {name}", index + 1);
	let mut wide: Vec<u32> = text.chars().map(|c| c as u32).collect();
	wide.push(0);
	(wide, text)
}

/// Every pack under `plugins/bgms/`, in name order so the sets keep their
/// numbering between runs -- which matters, because the chosen set is what
/// goes in the card.
unsafe fn packs() -> Vec<Set> {
	let mut folders: Vec<std::path::PathBuf> = match std::fs::read_dir("plugins/bgms") {
		Ok(entries) => entries
			.filter_map(|entry| entry.ok())
			.map(|entry| entry.path())
			.filter(|path| path.is_dir())
			.collect(),
		Err(_) => return Vec::new(),
	};
	folders.sort();

	let mut sets = Vec::new();
	for folder in folders {
		let name = folder
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default();
		let config = folder.join("mod.toml");
		let image = folder.join("mod.png");
		// Read as bytes and convert lossily: dump assets have taught us not
		// to trust an asset's claimed encoding, and a pack is no different.
		let Ok(bytes) = std::fs::read(&config) else {
			platform::log(&format!("bgm: {name} has no mod.toml, skipped"));
			continue;
		};
		let text = String::from_utf8_lossy(&bytes);
		let pack: PackFile = match toml::from_str(&text) {
			Ok(pack) => pack,
			Err(err) => {
				platform::log(&format!("bgm: {name}/mod.toml does not parse ({err})"));
				continue;
			}
		};
		if pack.song.is_empty() {
			platform::log(&format!("bgm: {name} lists no songs, skipped"));
			continue;
		}

		let mut songs = Vec::new();
		for song in pack.song.iter() {
			let path = folder.join(&song.file);
			if !path.is_file() {
				platform::log(&format!(
					"bgm: {name} wants {}, which is not there",
					path.display()
				));
				continue;
			}
			// Numbered by what has loaded rather than by position in the
			// file, so a song whose file is missing does not leave a gap in
			// the list the player sees.
			let (title, text) = wide_title(songs.len(), &song.name);
			songs.push(Song {
				path: std::ffi::CString::new(path.to_string_lossy().as_ref()).unwrap_or_default(),
				title,
				name: text,
			});
		}
		if songs.is_empty() {
			platform::log(&format!("bgm: {name} has no playable songs, skipped"));
			continue;
		}
		platform::log(&format!("bgm: {name}, {} song(s)", songs.len()));
		sets.push(Set {
			name,
			image: image
				.is_file()
				.then(|| std::ffi::CString::new(image.to_string_lossy().as_ref()).ok())
				.flatten(),
			songs,
			stories: pack.stories,
		});
	}
	sets
}

// ------------------------------------------------------------------ playback

/// A song index in `set`, 1-based, chosen without a dependency on `rand`.
///
/// xorshift32 seeded from the clock. Picking a race track does not need a
/// good generator, and the alternative is a crate in the dependency tree for
/// one call.
unsafe fn random_index(songs: usize) -> i32 {
	static mut STATE: u32 = 0;
	if STATE == 0 {
		STATE = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0x2545_F491, |d| d.subsec_nanos() | 1);
	}
	STATE ^= STATE << 13;
	STATE ^= STATE >> 17;
	STATE ^= STATE << 5;
	(STATE as usize % songs) as i32 + 1
}

unsafe fn random_song(set: i32) -> i32 {
	let Some(songs) = SETS.get(set as usize).map(|set| set.songs.len()) else {
		return 0;
	};
	encode(set, random_index(songs))
}

unsafe fn song(id: i32) -> Option<&'static Song> {
	let index = index_of(id);
	if index < 1 {
		return None;
	}
	SETS.get(set_of(id) as usize)?.songs.get(index as usize - 1)
}

/// Points `BGM_TABLE`'s scratch entry at `song`, so both reads inside
/// `clAudioStream::play` -- the header through `clWaveFile::loadFile` and the
/// `fopen` that streams the body -- come from the same file.
unsafe fn aim_scratch(song: &Song) {
	let Some(entry) = entry(SCRATCH) else {
		return;
	};
	let path = song.path.as_ptr() as usize as u32;
	let title = song.title.as_ptr() as usize as u32;
	platform::write_memory(entry as *mut (), &path.to_le_bytes());
	platform::write_memory(entry.add(8) as *mut (), &title.to_le_bytes());
}

static mut ORIGINAL_PLAY: *const () = std::ptr::null();

unsafe extern "C" fn play(this: *mut c_void, id: c_int) -> c_int {
	if id < ENCODED {
		return call_game(ORIGINAL_PLAY, (this, id));
	}
	let Some(song) = song(id) else {
		REPORTED.say(format!("bgm: nothing to play for id {id}"));
		return call_game(ORIGINAL_PLAY, (this, id));
	};
	REPORTED.say(format!(
		"bgm: {} from {}",
		song.name,
		song.path.to_string_lossy()
	));
	aim_scratch(song);
	call_game(ORIGINAL_PLAY, (this, SCRATCH))
}

// --------------------------------------------------------------- the menu
//
// The select screen's five lookups. Four return an int and are replaced
// outright; the fifth returns a std::wstring and goes through the trampoline
// below.

unsafe extern "C" fn select_id(set: c_int, index: c_int) -> c_int {
	if index == 0 {
		random_song(set)
	} else {
		encode(set, index)
	}
}

unsafe extern "C" fn set_from_id(id: c_int) -> c_int {
	set_of(id)
}

unsafe extern "C" fn select_index(_set: c_int, id: c_int) -> c_int {
	index_of(id)
}

unsafe extern "C" fn select_next(set: c_int, index: c_int) -> c_int {
	let songs = SETS.get(set as usize).map_or(0, |set| set.songs.len());
	next_index(songs, index)
}

static mut ORIGINAL_ICON: *const () = std::ptr::null();

/// The icon for a set. Custom sets have no icon of their own, so they borrow
/// set 0's and answer the logo request with their own image instead.
unsafe extern "C" fn select_icon(set: c_int, id: c_int) -> c_int {
	SHOWN_SET = set;
	let native = if set as usize >= MENU_SETS { 0 } else { set };
	call_game(ORIGINAL_ICON, (native, id))
}

static mut ORIGINAL_LOAD_IMAGE: *const () = std::ptr::null();

/// The pack's `mod.png` in place of the maxi3 logo, while a custom set is the
/// one on screen.
unsafe extern "C" fn load_image(this: *mut c_void, path: *const c_char) -> c_int {
	if path.is_null() || (SHOWN_SET as usize) < MENU_SETS {
		return call_game(ORIGINAL_LOAD_IMAGE, (this, path));
	}
	let asked = CStr::from_ptr(path).to_string_lossy();
	if asked.ends_with("menu_p_sound_logo_maxi3.png") {
		if let Some(image) = SETS.get(SHOWN_SET as usize).and_then(|set| set.image.as_ref()) {
			return call_game(ORIGINAL_LOAD_IMAGE, (this, image.as_ptr()));
		}
	}
	call_game(ORIGINAL_LOAD_IMAGE, (this, path))
}

// ------------------------------------------------- the std::wstring returns
//
// `nsMenuCheck::getBgmSelectName` returns a std::wstring by value, which on
// the ABI the game was built with means a hidden pointer to the caller's
// storage as the first argument *that the callee pops*: the original ends in
// `ret 0x4`. Rust's `extern "C"` always returns with a plain `ret`, and the
// one Rust convention that happens to match -- i686 System V returns every
// struct through sret -- is Linux-only, so relying on it would leave the
// Windows build quietly corrupting the caller's stack by four bytes a call.
//
// So the hook is a naked trampoline that takes the pointer as an ordinary
// argument and ends in `ret 4`, the same kind of ABI repair `shim/call.c`
// does for calls in the other direction. It realigns the stack for the same
// reason `call_game` does: gcc hands us an esp that is 16-byte aligned at its
// call site, and Rust's own code expects to be entered the same way.

static mut WIDE_STRING: *const () = std::ptr::null();

/// Builds `std::wstring(text)` into the caller's storage.
///
/// The allocator argument is a reference to an empty class, so the game's own
/// call passes whatever is convenient and so does this one.
unsafe fn write_wstring(out: *mut c_void, text: *const u32) {
	if WIDE_STRING.is_null() || out.is_null() {
		return;
	}
	let _: *mut c_void = call_game(WIDE_STRING, (out, text, std::ptr::null::<c_void>()));
}

/// Deliberately not `#[no_mangle]`: an exported symbol forces the
/// trampoline's `call` through the PLT, and a direct `R_386_PC32` against one
/// is not allowed in the Linux build's shared object.
unsafe extern "C" fn select_name(out: *mut c_void, set: c_int, index: c_int) {
	let fallback: Vec<u32> = "RANDOM\0".chars().map(|c| c as u32).collect();
	let title = if index == 0 {
		fallback.as_ptr()
	} else {
		match SETS
			.get(set as usize)
			.and_then(|set| set.songs.get(index as usize - 1))
		{
			Some(song) => song.title.as_ptr(),
			None => fallback.as_ptr(),
		}
	};
	write_wstring(out, title);
}

#[unsafe(naked)]
unsafe extern "C" fn select_name_trampoline() {
	core::arch::naked_asm!(
		"push ebp",
		"mov ebp, esp",
		"and esp, -16",
		"sub esp, 16",
		"mov eax, [ebp + 8]",
		"mov [esp], eax",
		"mov eax, [ebp + 12]",
		"mov [esp + 4], eax",
		"mov eax, [ebp + 16]",
		"mov [esp + 8], eax",
		"call {name}",
		// The game reads the string back out of eax as well as out of the
		// storage it passed, so answer with the pointer it gave us.
		"mov eax, [ebp + 8]",
		"leave",
		"ret 4",
		name = sym select_name,
	)
}

// ------------------------------------------------------------------ the card
//
// The chosen set lives in the card at +0x16D, and the arcade only knows about
// three of them. `plugins/audio.toml` holds the choice instead, and the byte
// is put back to what the card really said before anything writes it out, so
// a card carrying a custom set is never handed to a cabinet that would read
// it as something else.

fn chosen_set() -> Option<usize> {
	#[derive(serde::Deserialize)]
	struct Chosen {
		bgm: String,
	}
	let text = std::fs::read_to_string("plugins/audio.toml").ok()?;
	let chosen: Chosen = toml::from_str(&text).ok()?;
	unsafe { SETS.iter().position(|set| set.name == chosen.bgm) }
}

static mut ORIGINAL_CARD_LOAD: *const () = std::ptr::null();

unsafe extern "C" fn card_load(
	data: *mut u8,
	a2: *const c_char,
	a3: *mut c_char,
	a4: bool,
) -> *const c_void {
	let result: *const c_void = call_game(ORIGINAL_CARD_LOAD, (data, a2, a3, a4 as c_int));
	if data.is_null() {
		return result;
	}
	CARD_SET = data.add(CARD_BGM).read();
	if let Some(set) = chosen_set() {
		platform::write_memory(data.add(CARD_BGM) as *mut (), &[set as u8]);
	}
	result
}

static mut ORIGINAL_CARD_SAVE: *const () = std::ptr::null();

unsafe extern "C" fn card_save(data: *mut u8, a2: bool, a3: *const u8) -> *const u8 {
	if data.is_null() {
		return call_game(ORIGINAL_CARD_SAVE, (data, a2 as c_int, a3));
	}
	let slot = data.add(CARD_BGM);
	let ours = slot.read();
	slot.write(CARD_SET);
	let result: *const u8 = call_game(ORIGINAL_CARD_SAVE, (data, a2 as c_int, a3));
	slot.write(ours);
	result
}

// ------------------------------------------------------------- story mode

static mut ORIGINAL_STORY_BGM: *const () = std::ptr::null();

/// The song a story episode plays. A pack can name one per episode in its
/// `stories` list; without one it gets a random song from the pack.
unsafe extern "C" fn story_bgm(episode: *mut c_int, a2: c_int, set: c_int) -> c_int {
	if a2 > 0 {
		return -1;
	}
	if (set as usize) < MENU_SETS || episode.is_null() {
		return call_game(ORIGINAL_STORY_BGM, (episode, a2, set));
	}
	let Some(pack) = SETS.get(set as usize) else {
		return call_game(ORIGINAL_STORY_BGM, (episode, a2, set));
	};
	let Some(stories) = &pack.stories else {
		return random_song(set);
	};
	let wanted = stories.get(episode.read() as usize);
	let found = wanted.and_then(|wanted| {
		pack.songs
			.iter()
			.position(|song| song.name.ends_with(wanted.as_str()))
	});
	match found {
		Some(index) => encode(set, index as i32 + 1),
		None => random_song(set),
	}
}

// ------------------------------------------------------------------- startup

pub unsafe fn init() {
	if !CONFIG.custom_bgm {
		return;
	}
	if GAME_VERSION.major != GameMajor::W3P {
		platform::log("bgm: the table addresses are 3DX+'s, and this is not it");
		return;
	}

	SETS = native_sets();
	let native = SETS.len();
	SETS.extend(packs());
	if SETS.len() == native {
		platform::log(
			"bgm: no packs in plugins/bgms, so the BGM select screen is left alone. A pack is a \
			 folder there holding mod.toml and mod.png.",
		);
		SETS = Vec::new();
		return;
	}
	platform::log(&format!(
		"bgm: {native} set(s) from the game and {} from plugins/bgms",
		SETS.len() - native
	));

	// The clamp, verified before it is written: three bytes that must be the
	// `cmovge eax, edx` the roadmap records, or this is not the build the
	// address belongs to.
	let clamp = std::slice::from_raw_parts(CLAMP as *const u8, CLAMP_BYTES.len());
	if clamp == CLAMP_BYTES {
		platform::write_memory(CLAMP as *mut (), &[0x90; 3]);
	} else {
		platform::log(&format!(
			"bgm: the set clamp at {CLAMP:#x} is {clamp:02x?}, not {CLAMP_BYTES:02x?} -- leaving \
			 it alone, so a custom set will not survive being written to a card"
		));
	}

	WIDE_STRING = hook::get_symbol("_ZNSbIwSt11char_traitsIwESaIwEEC1EPKwRKS1_") as *const ();
	if WIDE_STRING.is_null() {
		// It lives in libstdc++, not in `main`, so this is a loader problem
		// rather than a wrong name: see the broken-symlink note in CLAUDE.md.
		platform::log("bgm: the std::wstring constructor is missing, so song names will be blank");
	}

	ORIGINAL_PLAY = hook::hook_symbol(
		"_ZN13clAudioStream4playEN7nsAudio9emBgmTypeE",
		play as *const (),
	);
	hook::hook_symbol("_ZN11nsMenuCheck14getBgmSelectIdEii", select_id as *const ());
	hook::hook_symbol(
		"_ZN11nsMenuCheck15getBgmSetFromIdEN7nsAudio9emBgmTypeE",
		set_from_id as *const (),
	);
	hook::hook_symbol(
		"_ZN11nsMenuCheck15getBgmSelectIdxEiN7nsAudio9emBgmTypeE",
		select_index as *const (),
	);
	hook::hook_symbol(
		"_ZN11nsMenuCheck16getBgmSelectNextEii",
		select_next as *const (),
	);
	hook::hook_symbol(
		"_ZN11nsMenuCheck16getBgmSelectNameEii",
		select_name_trampoline as *const (),
	);
	ORIGINAL_ICON = hook::hook_symbol(
		"_ZN11nsMenuCheck16getBgmSelectIconEii",
		select_icon as *const (),
	);
	hook::hook_symbol(
		"_ZN11nsMenuCheck10getBgmIconEiN7nsAudio9emBgmTypeE",
		select_icon as *const (),
	);
	ORIGINAL_LOAD_IMAGE = hook::hook_symbol("_ZN3Gap3Gfx7igImage4loadEPKc", load_image as *const ());
	ORIGINAL_CARD_LOAD = hook::hook_symbol(
		"_ZN14clV386CardData6assignEPKcPcb",
		card_load as *const (),
	);
	ORIGINAL_CARD_SAVE =
		hook::hook_symbol("_ZN14clV386CardData4dataEbPc", card_save as *const ());
	ORIGINAL_STORY_BGM =
		hook::hook_symbol("_ZNK14clEffStoryType6getBgmEii", story_bgm as *const ());
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The encoding has to round-trip, because the card stores the set and the
	/// menu stores the index, and `audio_3dxp` wrote cards with these same
	/// numbers.
	#[test]
	fn a_set_and_index_survive_the_round_trip() {
		for set in 0..8 {
			for index in 1..25 {
				let id = encode(set, index);
				assert!(id >= ENCODED, "set {set} index {index} encodes below a game id");
				assert_eq!(set_of(id), set);
				assert_eq!(index_of(id), index);
			}
		}
	}

	/// The game's own ids have to stay the game's own, or `play` forwards a
	/// custom song as a native one and vice versa.
	#[test]
	fn the_games_own_ids_are_left_below_the_encoding() {
		assert!(BGM_ENTRIES < ENCODED as usize);
		assert_eq!(encode(0, 1), ENCODED + 1);
		// RANDOM and "nothing selected" both mean index 0.
		assert_eq!(index_of(RANDOM_ID), 0);
		assert_eq!(index_of(-1), 0);
		assert_eq!(set_of(-1), 0);
	}

	#[test]
	fn the_cursor_wraps_past_the_last_song_to_random() {
		assert_eq!(next_index(3, 0), 1);
		assert_eq!(next_index(3, 2), 3);
		assert_eq!(next_index(3, 3), 0);
		// A set that failed to load must not walk off the end.
		assert_eq!(next_index(0, 0), 0);
	}

	/// A custom title is numbered the way the game numbers its own, so the
	/// list reads the same whichever set is on screen.
	#[test]
	fn a_custom_title_looks_like_a_shipped_one() {
		let (wide, text) = wide_title(0, "Stay where you are");
		assert_eq!(text, "1 Stay where you are");
		assert_eq!(*wide.last().unwrap(), 0, "the title is not NUL-terminated");
		assert_eq!(wide.len(), text.chars().count() + 1);
		assert_eq!(wide_title(19, "Phantom of Blue").1, "20 Phantom of Blue");
	}

	/// The scratch entry has to be inside the table, since its address is
	/// computed rather than read.
	#[test]
	fn the_scratch_entry_is_in_the_table() {
		assert!(SCRATCH >= 0 && (SCRATCH as usize) < BGM_ENTRIES);
		assert_eq!(BGM_TABLE + BGM_ENTRIES * BGM_ENTRY, 0x0932_1260 + 111 * 12);
	}
}
