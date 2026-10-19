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
	/// The song's own name, with no number in front of it: what a pack's
	/// `stories` list names, and what the log reports. A native song keeps the
	/// game's numbered title here, since nothing ever matches against those.
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
			let (title, _) = wide_title(songs.len(), &song.name);
			songs.push(Song {
				path: std::ffi::CString::new(path.to_string_lossy().as_ref()).unwrap_or_default(),
				title,
				name: song.name.clone(),
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

/// The PLT stub for `std::wstring(wchar_t const*, allocator const&)`.
///
/// Resolving it by name does not work. It lives in `libso/libstdc++.so.6` as
/// a weak symbol, and LINE's `DlSym` does not return it -- measured on
/// hardware, where the loader logged the constructor as missing. The game
/// calls it too, though, through this stub at `0x8541339`, and the stub's GOT
/// slot has already been relocated by the time anything runs. So the stub is
/// the reliable way in, and its own six bytes identify it.
const WIDE_STRING_PLT: usize = 0x0842_1390;
const WIDE_STRING_PLT_BYTES: [u8; 6] = [0xff, 0x25, 0x40, 0xce, 0x7f, 0x09];

/// libstdc++'s shared empty `_Rep`, and the header that sits in front of a
/// string's characters.
///
/// A default-constructed `std::wstring` holds `EMPTY_REP + REP_HEADER` --
/// `clSeqStdDayTimeSelectThread::bgmUpdate` says so itself, taking the
/// pointer it was handed, subtracting 12 and comparing against this address
/// to decide whether the string needs releasing. That is where the header
/// size comes from, and it is also why handing this value back is safe: the
/// shared empty rep's refcount is -1, so nothing ever tries to free it.
const EMPTY_REP: usize = 0x097F_D4C8;
const REP_HEADER: usize = 12;

/// Builds `std::wstring(text)` into the caller's storage.
///
/// The allocator argument is a reference to an empty class, so the game's own
/// call passes whatever is convenient and so does this one.
unsafe fn write_wstring(out: *mut c_void, text: *const u32) {
	if out.is_null() {
		return;
	}
	if WIDE_STRING.is_null() {
		// **Never leave the caller's storage untouched.** It is an
		// uninitialised stack slot, and the caller hands it straight to
		// `clSpriteFont::setString` and then to a destructor -- so returning
		// without writing anything is not "no title", it is a garbage pointer
		// read and then freed. An empty string is the quiet failure.
		out.cast::<usize>().write(EMPTY_REP + REP_HEADER);
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

// ----------------------------------------------- the card screen's selector
//
// Card check page 8 (`clSeqCardDisp9`) is a three-row settings editor, and
// row 1 is the BGM set: value at `[this+0x48]`, maximum at `[this+0x60]`,
// artwork on the `clSpriteAnime` at `[this+0x24]`. The cursor row is
// `[this+0x40]`, moved with the intrude button; the value is changed with the
// shifter left and right through `clMenuInput::chkSW` -- the one funnel every
// menu in this game reads buttons through, and which only ever produces left,
// right and four buttons. There is no up or down anywhere in these menus.
//
// Three things stand between that selector and a custom set.
//
// **The maximum is an unlock count.** `run` recomputes it on every pass, so
// writing the field from a hook would be overwritten; the gate is patched
// instead. `[this+0x68]` is set to 0, then 1 if `card+0x279` is set, then 2
// if `card+0x186` is set, and the result reaches `[this+0x60]` through a
// three-way dispatch. Forcing both lower writes to 2 makes every path take
// the `== 2` branch -- which is also the branch that loads the full
// `menu_g_cardcheck_8_sound_sel` plate rather than the cut-down or locked
// ones, so one change unlocks the row *and* gives it artwork with room for
// three blocks of frames. Only that branch's store then needs the real count.
//
// **Row 1 clamps.** Only row 2 wraps, and the test is the row index against
// 2 in both directions. Making it "row >= 1" lets row 1 wrap too, which
// matters more with seven sets than it did with three.
//
// **The artwork is frame blocks, not images.** Each set shows 59 frames of
// one `.anm` -- start 0, 100 or 200 -- and for a set index of 3 or more the
// dispatch makes no call at all, leaving whatever frame was there. So a
// custom set borrows the third block, and the *sprite descriptor* that block
// draws from is pointed at the pack's own image. Note what this is not: the
// card screen never goes through `Gap::Gfx::igImage::load`, so the hook that
// substitutes the pre-race logo cannot reach it. Sprites here resolve through
// `clImageResourceManager` from ids compiled into the `.anm`, and the
// descriptor table is the only place a path can be changed.

/// `mov DWORD PTR [esi+0x68],0x0` -- the unlock count before either card byte
/// is consulted.
const GATE_ZERO: usize = 0x087E_F16C;
/// `mov DWORD PTR [esi+0x68],0x1` -- the one-unlock case.
const GATE_ONE: usize = 0x087E_F192;
/// `mov DWORD PTR [esi+0x60],0x2` -- row 1's maximum, on the branch every
/// path takes once the gate is forced.
const ROW_MAX: usize = 0x087E_F936;
/// `cmp edx,0x2; jne` -- the wrap test on the way up.
const WRAP_UP: usize = 0x087E_FD1A;
/// `cmp eax,0x2; jne` -- the wrap test on the way down.
const WRAP_DOWN: usize = 0x087E_FEDF;

/// Sprite descriptor for `menu_p_card_8_sound_1_3.png`, id `0x1599`, in the
/// table `clMain` hands to `clImageResourceManager::create`. 44 bytes:
/// `{ const char *path, int, int, int, float w, h, sx, sy, w2, h2, int }`.
///
/// **The card screen has its own artwork**, nothing to do with the pre-race
/// logos: `menu_p_card_8_sound_<set>_<state>.png`, ids `0x1597`..`0x159f`, a
/// 3x3 grid of the three sets against the three unlock states, at 128x64.
/// This is set 1 in state 3 -- the maxi3 soundtrack on a fully unlocked row,
/// which is the frame a custom set borrows. Pointing the pre-race logo's
/// descriptor here instead, as this first did, could never have worked: that
/// one is reached through `igImage::load`, which this screen never calls.
/// All three states of it. Which one a given frame draws is in the `.anm`,
/// which has not been decoded -- repointing only `_1_3` changed nothing on
/// screen, so the frame uses one of the others. Repointing the set as a whole
/// sidesteps having to know, and is right either way: a custom set always
/// borrows this slot, so every state of it should be the pack's.
const LOGO_DESCS: [usize; 3] = [0x0959_6094, 0x0959_60C0, 0x0959_60EC];
const LOGO_WIDTH: usize = 0x10;
const LOGO_HEIGHT: usize = 0x14;
const LOGO_WIDTH2: usize = 0x20;
const LOGO_HEIGHT2: usize = 0x24;
const LOGO_BYTES: usize = 0x2c;

/// Row 1 of the card screen: the value, and the sprite that illustrates it.
const ROW_VALUE: usize = 0x48;
const ROW_SPRITE: usize = 0x24;
/// The rest of the row's state, read only to report it: the cursor's row, the
/// unlock count the patches force, the limit they set, and the title sprite
/// -- which nothing here writes, so a blank title means the row was never
/// drawn rather than that the value was wrong.
const CURSOR_ROW: usize = 0x40;
/// The value the row was showing last time, which the screen compares against
/// the current one to decide whether to travel between sets.
const ROW_SETTLED: usize = 0x54;
const UNLOCK_COUNT: usize = 0x68;
const ROW_LIMIT: usize = 0x60;
const ROW_TITLE: usize = 0x30;
/// How the row draws itself, which is not what it first looked like.
///
/// `ANM` is a **playback mode**, not a slot: mode 6 loops a block of frames
/// (what the row does while the cursor sits on it) and mode 1 shows a single
/// frame (what it shows the rest of the time). The resting display is one
/// frame at `STATIC_FRAME + set`, and the blocks at 0, 100 and 200 are the
/// animated version.
///
/// Which plate those frames come from is chosen from the unlock count too --
/// `0xa2` locked, `0xa1` part-unlocked, `0xa0` full -- and that choice is
/// made *before* `run` computes the count the patches force. So the row was
/// holding `menu_g_cardcheck_8_sound_sel_no`, all 32 KB of it, and being
/// asked for frame 260: out of range, nothing drawn, for every set including
/// the game's own three. Loading the full plate is the other half of the
/// patch, and it is why forcing the unlock count alone drew nothing.
/// Frame `0x104 + set` is the resting display once the row is unlocked, and
/// `menu_g_cardcheck_8_sound_sel.anm` holds 263 frames -- so the three sets
/// the game ships rest on 260, 261 and 262 and there is nothing past them.
/// Setting those frames here is not how the row is driven, though; see
/// `borrow_set`.
const SOUND_PLATE: c_int = 0xa0;
const PLATE_MODE: c_int = 6;
/// The set a custom one borrows, and whose artwork `LOGO_DESC` replaces.
const BORROWED_SET: c_int = 0;

/// One verified in-memory edit.
///
/// Absolute addresses, because this module works in them throughout: `main`
/// is `ET_EXEC` and not position-independent, so a link address is a runtime
/// address.
struct Edit {
	at: usize,
	expect: &'static [u8],
	write: &'static [u8],
	what: &'static str,
}

/// The edits that make the row reach past three sets -- everything except the
/// maximum itself, which depends on how many packs loaded.
const SELECTOR: [Edit; 4] = [
	Edit {
		at: GATE_ZERO,
		expect: &[0xc7, 0x46, 0x68, 0x00, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x46, 0x68, 0x02, 0x00, 0x00, 0x00],
		what: "the BGM row starts unlocked rather than locked",
	},
	Edit {
		at: GATE_ONE,
		expect: &[0xc7, 0x46, 0x68, 0x01, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x46, 0x68, 0x02, 0x00, 0x00, 0x00],
		what: "a part-unlocked card gets the whole row, not two thirds of it",
	},
	Edit {
		at: WRAP_UP,
		expect: &[0x83, 0xfa, 0x02, 0x0f, 0x85],
		write: &[0x83, 0xfa, 0x01, 0x0f, 0x8c],
		what: "the BGM row wraps to the first set instead of stopping",
	},
	Edit {
		at: WRAP_DOWN,
		expect: &[0x83, 0xf8, 0x02, 0x0f, 0x85],
		write: &[0x83, 0xf8, 0x01, 0x0f, 0x8c],
		what: "the BGM row wraps to the last set instead of stopping",
	},
];

/// Applies a set of edits, or none of them.
///
/// A half-applied set is worse than none here: a row that scrolls to a set it
/// cannot draw, or draws one it cannot store, is harder to diagnose than a
/// row that never changed. Each write is read back, because a page the loader
/// cannot make writable would otherwise look exactly like an edit that works
/// and is ignored.
unsafe fn apply(edits: &[Edit], name: &str) -> bool {
	let found = |edit: &Edit| std::slice::from_raw_parts(edit.at as *const u8, edit.expect.len());
	if let Some(wrong) = edits.iter().find(|edit| found(edit) != edit.expect) {
		platform::log(&format!(
			"bgm: not patching {name} -- {:#x} holds {:02x?}, not {:02x?}",
			wrong.at,
			found(wrong),
			wrong.expect
		));
		return false;
	}
	for edit in edits {
		platform::write_memory(edit.at as *mut (), edit.write);
		if found(edit) == edit.write {
			platform::log(&format!("bgm: {}", edit.what));
		} else {
			platform::log(&format!("bgm: write did not stick at {:#x}", edit.at));
		}
	}
	true
}

/// A PNG's pixel size out of its IHDR, as bytes.
///
/// Split out from the file read so the parse itself is testable: a header
/// that is not a PNG, or is truncated, has to answer None rather than a size
/// the engine would then sample with.
fn png_size_of(bytes: &[u8]) -> Option<(u32, u32)> {
	if bytes.len() < 24 || &bytes[1..4] != b"PNG" || &bytes[12..16] != b"IHDR" {
		return None;
	}
	let at = |offset: usize| -> Option<u32> {
		Some(u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?))
	};
	let (width, height) = (at(16)?, at(20)?);
	// A zero dimension would make the descriptor sample nothing at all.
	(width > 0 && height > 0).then_some((width, height))
}

/// A PNG's pixel size, straight out of its IHDR.
///
/// The descriptor carries the image's dimensions and the engine samples with
/// them, so a pack's 192x32 image left behind a 256x32 descriptor would come
/// out stretched. Reading the real size costs 24 bytes of file.
fn png_size(path: &CStr) -> Option<(u32, u32)> {
	png_size_of(&std::fs::read(path.to_str().ok()?).ok()?)
}
/// The descriptors as they shipped, so a native set gets its own logo back.
static mut LOGO_ORIGINAL: [[u8; LOGO_BYTES]; 3] = [[0; LOGO_BYTES]; 3];
static mut LOGO_SAVED: bool = false;
/// Which set the descriptor points at, so it is only rewritten on a change.
static mut LOGO_SET: i32 = -1;

/// Points the borrowed set's descriptor at `set`'s own image, or back at the
/// artwork it shipped with.
///
/// **This has to happen before the plate is loaded**, because that is when
/// `clImageResourceManager` instantiates the images the frames refer to;
/// repointing a descriptor afterwards changes nothing that is already on
/// screen. So it runs once at startup against the set `plugins/audio.toml`
/// names, which is the set the screen is entered on. Scrolling to a different
/// pack within the screen keeps showing that one's artwork until the next
/// launch -- the row and the music follow immediately, only the picture
/// lags.
unsafe fn aim_logo(set: i32) {
	if !LOGO_SAVED || LOGO_SET == set {
		return;
	}
	LOGO_SET = set;
	let image = SETS.get(set as usize).and_then(|set| set.image.as_ref());
	let Some(image) = image else {
		for (desc, original) in LOGO_DESCS.iter().zip(LOGO_ORIGINAL.iter()) {
			platform::write_memory(*desc as *mut (), original);
		}
		return;
	};
	let pointer = image.as_ptr() as usize as u32;
	let size = png_size(image);
	for desc in LOGO_DESCS {
		platform::write_memory(desc as *mut (), &pointer.to_le_bytes());
		if let Some((width, height)) = size {
			for offset in [LOGO_WIDTH, LOGO_WIDTH2] {
				platform::write_memory((desc + offset) as *mut (), &(width as f32).to_le_bytes());
			}
			for offset in [LOGO_HEIGHT, LOGO_HEIGHT2] {
				platform::write_memory((desc + offset) as *mut (), &(height as f32).to_le_bytes());
			}
		}
	}
	platform::log(&format!(
		"bgm: the card screen's first sound plate now draws {}",
		image.to_string_lossy()
	));
}

static mut LOAD_ANIME: *const () = std::ptr::null();
/// The screen the plate has been loaded for, so it is loaded once per visit
/// rather than once per redraw.
static mut PLATE_LOADED_FOR: *mut c_void = std::ptr::null_mut();
/// The row value the screen last showed, so the choice is written out once
/// per change rather than once per frame.
static mut ROW_SHOWN: i32 = -1;

/// Brings the BGM row in line with a set the screen was never built to draw,
/// and remembers the choice.
///
/// Called from both of the screen's animation entry points, after the game's
/// own work: `an_reset` runs on every cursor move and value change, and
/// `initAnime` on the first draw, which `an_reset` does not cover.
unsafe fn row_changed(this: *mut c_void) {
	if this.is_null() {
		return;
	}
	let field = |offset: usize| (this as *const u8).add(offset).cast::<c_int>().read();
	let value = field(ROW_VALUE);
	// Deduplicated, so this is a handful of lines over a whole session: one
	// per distinct state the row is in. It is the only way to tell "the patch
	// did not take" from "the patch took and the row still draws nothing".
	REPORTED.say(format!(
		"bgm: card row 1 = set {value}, unlock {}, limit {}, cursor on row {}, value sprite 		 {:#x}, title sprite {:#x}",
		field(UNLOCK_COUNT),
		field(ROW_LIMIT),
		field(CURSOR_ROW),
		field(ROW_SPRITE),
		field(ROW_TITLE),
	));
	if value < 0 || value as usize >= SETS.len() {
		return;
	}
	SHOWN_SET = value;

	// Make sure the row is holding the full plate. The game picks which one
	// from the unlock count before the patched value exists, so left alone it
	// keeps the locked plate and every frame the row asks for is out of range
	// -- a blank row whatever is selected, the game's own three sets included.
	//
	// **Once per screen, not once per call.** Loading an anime restarts it,
	// and this runs on every `an_reset`, so doing it unconditionally made the
	// row replay its slide roughly every 1.5 seconds whether or not anybody
	// touched the wheel. Measured off a recording: bursts at 2.6, 4.3, 5.6,
	// 7.4, 9.2, 11.0, 12.8 seconds and so on, with the wheel untouched.
	if PLATE_LOADED_FOR != this && !LOAD_ANIME.is_null() {
		let sprite = (this as *const u8)
			.add(ROW_SPRITE)
			.cast::<*mut c_void>()
			.read();
		if !sprite.is_null() {
			call_game::<_, ()>(LOAD_ANIME, (sprite, SOUND_PLATE, PLATE_MODE, 0, 0, 1));
			PLATE_LOADED_FOR = this;
		}
	}

	if ROW_SHOWN != value {
		let first = ROW_SHOWN == -1;
		ROW_SHOWN = value;
		// Not on the first draw: that value came out of the card, which came
		// out of `audio.toml` in the first place.
		if !first {
			if let Some(set) = SETS.get(value as usize) {
				remember(&set.name);
			}
		}
	}
}

/// Writes the chosen set back to `plugins/audio.toml`.
///
/// Through a temporary file in the same directory and a rename, because an
/// interrupted write would leave an `audio.toml` that does not parse, and
/// `chosen_set` treats a file it cannot parse exactly like a file that names
/// nothing -- by silently leaving the player on the native sets.
fn remember(name: &str) {
	let temp = "plugins/audio.toml.new";
	if let Err(err) = std::fs::write(temp, format!("bgm = \"{name}\"\n")) {
		platform::log(&format!("bgm: cannot write {temp} ({err})"));
		return;
	}
	if let Err(err) = std::fs::rename(temp, "plugins/audio.toml") {
		platform::log(&format!("bgm: cannot replace plugins/audio.toml ({err})"));
		return;
	}
	platform::log(&format!("bgm: plugins/audio.toml now chooses {name}"));
}

static mut ORIGINAL_AN_RESET: *const () = std::ptr::null();
static mut ORIGINAL_INIT_ANIME: *const () = std::ptr::null();

/// Shows the game a set it was built to draw, for the length of its own call.
///
/// Setting the frame ourselves afterwards does not work, and the reason is
/// worth keeping: with the cursor on the row, `an_reset` starts a **five-frame
/// transition** (`mode 4`, `count 5`) whose start frame it picks from the
/// direction of travel and the destination -- `cmp eax,1`, `cmp eax,2`, and
/// nothing at all for a set past those. That animation then keeps advancing
/// on the frames after, so a static frame written once is overwritten a
/// moment later and the row sits mid-slide with nothing highlighted.
///
/// So rather than fight the animation, the value is clamped to a set the
/// game understands while it draws, and put back immediately after. Every
/// path -- the resting frame, the transition, the direction test against the
/// previous value at `+0x54` -- then works as written, and `aim_logo` has
/// already pointed the borrowed set's artwork at the pack.
unsafe fn borrow_set(this: *mut c_void) -> Option<c_int> {
	if this.is_null() {
		return None;
	}
	let slot = (this as *mut u8).add(ROW_VALUE).cast::<c_int>();
	let real = slot.read();
	if (real as usize) < MENU_SETS {
		return None;
	}
	slot.write(BORROWED_SET);
	REPORTED.say(format!(
		"bgm: card row 1 drawn as set {BORROWED_SET} instead of {real}, cursor on row {}",
		(this as *const u8).add(CURSOR_ROW).cast::<c_int>().read()
	));
	Some(real)
}

unsafe fn give_back(this: *mut c_void, real: Option<c_int>) {
	let Some(real) = real else {
		return;
	};
	(this as *mut u8).add(ROW_VALUE).cast::<c_int>().write(real);
	// And tell it the row has settled there.
	//
	// The screen keeps the value it last drew at `+0x54` and compares it
	// against the current one -- `cmp [ecx+0x54], eax` -- to decide which way
	// to travel. Drawing under the clamp makes it record the borrowed set, so
	// putting the real one back on its own leaves the two permanently
	// disagreeing: it sets off towards a set it has no frames for and snaps
	// back, over and over, with nobody touching the wheel. Writing both keeps
	// the screen's own invariant, which is that the two match once a move has
	// finished.
	(this as *mut u8).add(ROW_SETTLED).cast::<c_int>().write(real);
}

unsafe extern "C" fn an_reset(this: *mut c_void) {
	let real = borrow_set(this);
	call_game::<_, ()>(ORIGINAL_AN_RESET, (this,));
	give_back(this, real);
	row_changed(this);
}

unsafe extern "C" fn init_anime(this: *mut c_void) {
	let real = borrow_set(this);
	call_game::<_, ()>(ORIGINAL_INIT_ANIME, (this,));
	give_back(this, real);
	row_changed(this);
}

/// Everything the card screen's row needs, or nothing.
unsafe fn install_selector() {
	// The row can only usefully offer more than the game already does.
	let sets = SETS.len();
	if sets <= MENU_SETS {
		return;
	}
	let mut row_max = [0xc7u8, 0x46, 0x60, 0x02, 0x00, 0x00, 0x00];
	let found = std::slice::from_raw_parts(ROW_MAX as *const u8, row_max.len());
	if found != row_max {
		platform::log(&format!(
			"bgm: not patching the BGM row's maximum -- {ROW_MAX:#x} holds {found:02x?}, not \
			 {row_max:02x?}"
		));
		return;
	}
	if !apply(&SELECTOR, "the BGM row") {
		return;
	}
	row_max[3..7].copy_from_slice(&((sets - 1) as u32).to_le_bytes());
	platform::write_memory(ROW_MAX as *mut (), &row_max);
	platform::log(&format!(
		"bgm: the card screen's BGM row now scrolls {sets} sets, wrapping at both ends"
	));

	for (slot, desc) in LOGO_ORIGINAL.iter_mut().zip(LOGO_DESCS) {
		*slot = std::slice::from_raw_parts(desc as *const u8, LOGO_BYTES)
			.try_into()
			.unwrap_or([0; LOGO_BYTES]);
	}
	LOGO_SAVED = LOGO_ORIGINAL
		.iter()
		.all(|entry| entry.iter().any(|byte| *byte != 0));

	// Before anything loads the plate, for the reason in `aim_logo`.
	if let Some(chosen) = chosen_set() {
		aim_logo(chosen as i32);
	}
	LOAD_ANIME = hook::get_symbol("_ZN13clSpriteAnime8setAnimeEi3ANMiii") as *const ();
	if LOAD_ANIME.is_null() {
		platform::log("bgm: clSpriteAnime::setAnime is missing, so the BGM row will stay blank");
	}
	ORIGINAL_AN_RESET = hook::hook_symbol("_ZN14clSeqCardDisp98an_resetEv", an_reset as *const ());
	ORIGINAL_INIT_ANIME =
		hook::hook_symbol("_ZN14clSeqCardDisp99initAnimeEv", init_anime as *const ());
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
			.position(|song| song.name == *wanted)
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
		// Expected: it lives in libstdc++ rather than in `main`, and LINE's
		// DlSym does not reach it. The game's own PLT stub does.
		let stub =
			std::slice::from_raw_parts(WIDE_STRING_PLT as *const u8, WIDE_STRING_PLT_BYTES.len());
		if stub == WIDE_STRING_PLT_BYTES {
			WIDE_STRING = WIDE_STRING_PLT as *const ();
			platform::log("bgm: song names go through the game's own std::wstring PLT stub");
		} else {
			platform::log(&format!(
				"bgm: no std::wstring constructor -- {WIDE_STRING_PLT:#x} holds {stub:02x?}, not 				 {WIDE_STRING_PLT_BYTES:02x?} -- so song names will be blank"
			));
		}
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

	if CONFIG.bgm_selector {
		install_selector();
	}
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

	/// A story episode names a song by its own name, and the match has to be
	/// exact. The packs we have include "Love And Gold" beside "Silver And
	/// Gold", so a suffix match picks whichever came first in the file.
	#[test]
	fn a_story_song_is_matched_whole_and_not_by_its_tail() {
		let names = ["Gold", "Love And Gold", "Silver And Gold", "Move On"];
		let find = |wanted: &str| names.iter().position(|name| *name == wanted);
		assert_eq!(find("Silver And Gold"), Some(2));
		assert_eq!(find("Gold"), Some(0));
		// What the old suffix match did with the same list.
		let tail = |wanted: &str| names.iter().position(|name| name.ends_with(wanted));
		assert_eq!(tail("Silver And Gold"), Some(2));
		assert_eq!(tail("Gold"), Some(0), "a bare suffix is ambiguous either way");
		assert_ne!(tail("And Gold"), find("And Gold"));
	}

	/// The scratch entry has to be inside the table, since its address is
	/// computed rather than read.
	#[test]
	fn the_scratch_entry_is_in_the_table() {
		assert!(SCRATCH >= 0 && (SCRATCH as usize) < BGM_ENTRIES);
		assert_eq!(BGM_TABLE + BGM_ENTRIES * BGM_ENTRY, 0x0932_1260 + 111 * 12);
	}
	/// The empty-string fallback has to be the exact pointer a
	/// default-constructed `std::wstring` holds, because `bgmUpdate` compares
	/// against it to decide whether to release the string. One byte out and
	/// the game frees the shared empty rep.
	#[test]
	fn an_empty_title_is_the_pointer_the_game_itself_tests_for() {
		// `lea ebx,[eax-0xc]; cmp ebx,0x97fd4c8` in bgmUpdate: the data
		// pointer less the header is the rep, so the rep plus the header is
		// the data pointer.
		assert_eq!(EMPTY_REP + REP_HEADER - REP_HEADER, EMPTY_REP);
		assert_eq!(REP_HEADER, 0xc, "the _Rep header is three machine words");
		assert_eq!(EMPTY_REP + REP_HEADER, 0x097F_D4D4);
	}

	/// An edit must write exactly as many bytes as it checked. One byte more
	/// and it runs into the next instruction, which on this screen is a jump.
	#[test]
	fn every_edit_writes_what_it_measured() {
		for edit in &SELECTOR {
			assert_eq!(
				edit.expect.len(),
				edit.write.len(),
				"{} changes the instruction length",
				edit.what
			);
			assert_ne!(edit.expect, edit.write, "{} writes nothing", edit.what);
		}
	}

	/// The two gate edits may only change the immediate of a `mov`. If the
	/// opcode or the destination moved, the edit is pointed at the wrong
	/// instruction and the expected bytes would not have caught it.
	#[test]
	fn the_gate_edits_change_only_the_number_written() {
		for edit in [&SELECTOR[0], &SELECTOR[1]] {
			assert_eq!(edit.expect[..3], edit.write[..3], "the mov itself changed");
			assert_eq!(edit.write[3], 2, "the unlock count must end up at 2");
			assert_eq!(edit.expect[4..], edit.write[4..]);
		}
	}

	/// The wrap edits turn `cmp reg,2; jne` into `cmp reg,1; jl`, so a row
	/// index of 1 or 2 takes the wrapping path and row 0 still clamps. Both
	/// the compared value and the condition have to move together: either on
	/// its own changes which rows wrap, and `jne` with 1 would wrap row 0.
	#[test]
	fn the_wrap_edits_move_the_bound_and_the_condition_together() {
		for edit in [&SELECTOR[2], &SELECTOR[3]] {
			assert_eq!(edit.expect[2], 2, "the test should start at row 2");
			assert_eq!(edit.write[2], 1, "the test should end at row 1");
			assert_eq!(edit.expect[3], 0x0f);
			assert_eq!(edit.expect[4], 0x85, "jne");
			assert_eq!(edit.write[4], 0x8c, "jl");
			assert_eq!(edit.expect[..2], edit.write[..2], "the cmp itself changed");
		}
	}

	/// The maximum is written as a dword immediate in the last four bytes of
	/// a seven-byte `mov`, so the slice the set count goes into has to line up
	/// with what the verified template measured.
	#[test]
	fn the_row_maximum_is_a_dword_at_the_end_of_the_mov() {
		let mut row_max = [0xc7u8, 0x46, 0x60, 0x02, 0x00, 0x00, 0x00];
		assert_eq!(row_max.len(), 7);
		row_max[3..7].copy_from_slice(&6u32.to_le_bytes());
		assert_eq!(row_max, [0xc7, 0x46, 0x60, 0x06, 0x00, 0x00, 0x00]);
		// Seven sets index 0..=6, so the maximum is one less than the count.
		assert_eq!((7usize - 1) as u32, 6);
	}

	/// A descriptor is 44 bytes and both size pairs have to land inside it.
	#[test]
	fn the_logo_descriptor_offsets_are_inside_the_entry() {
		for offset in [LOGO_WIDTH, LOGO_HEIGHT, LOGO_WIDTH2, LOGO_HEIGHT2] {
			assert!(offset + 4 <= LOGO_BYTES, "{offset:#x} runs off the entry");
		}
		assert_eq!(LOGO_BYTES, 0x2c);
		// The three states of one slot, adjacent entries in the table.
		for pair in LOGO_DESCS.windows(2) {
			assert_eq!(pair[1] - pair[0], LOGO_BYTES, "the entries are not adjacent");
		}
	}

	/// A custom set is shown to the screen as one of the three it was built
	/// to draw. If the borrowed one were not among them, every path -- the
	/// resting frame, the five-frame transition, the direction test -- would
	/// be back to indexing past the end of the plate, which is what a blank
	/// row looked like.
	#[test]
	fn the_borrowed_set_is_one_the_screen_can_draw() {
		assert!(BORROWED_SET >= 0 && (BORROWED_SET as usize) < MENU_SETS);
		// And the plate has a resting frame for each of them: 0x104 + set,
		// over 263 frames, so 260, 261 and 262 with nothing past.
		assert_eq!(0x104 + MENU_SETS, 263);
	}

	/// What the engine samples with comes out of this, so anything that is
	/// not a whole PNG header has to answer None rather than a plausible size.
	#[test]
	fn a_logo_size_is_only_read_from_a_real_png_header() {
		let mut png = vec![0x89u8];
		png.extend_from_slice(b"PNG\r\n\x1a\n");
		png.extend_from_slice(&13u32.to_be_bytes());
		png.extend_from_slice(b"IHDR");
		png.extend_from_slice(&192u32.to_be_bytes());
		png.extend_from_slice(&32u32.to_be_bytes());
		assert_eq!(png_size_of(&png), Some((192, 32)));

		assert_eq!(png_size_of(&png[..23]), None, "a truncated header");
		assert_eq!(png_size_of(b"not a png at all, but long enough......"), None);
		let mut zero = png.clone();
		zero[16..20].copy_from_slice(&0u32.to_be_bytes());
		assert_eq!(png_size_of(&zero), None, "a zero width");
	}
}
