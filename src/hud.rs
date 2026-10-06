//! Which console owns which sprite, and where in the 640-space layout it sits.
//!
//! Measurement only. Nothing here changes what the game draws; it exists to
//! produce the table that a later anchor-based nudge needs, and to check the
//! two assumptions that nudge would rest on.
//!
//! What is already established, so this does not have to re-derive it:
//!
//! - The 2D layer is laid out in 640x480 units, and `res.rs::widen` corrects
//!   the *proportions* at a non-4:3 render but not the *placement*, so the
//!   whole HUD sits in the middle with a margin each side.
//! - Positions reach a sprite from the `.anm` timeline, not from the setters:
//!   diffing tatoohanz's widescreen layouts against the dump's own shows x
//!   animated per keyframe, tens of thousands of values per file. Hooking
//!   `setPosition` would see almost none of it.
//! - The field offsets, read out of the dump's own 20-byte setters rather than
//!   assumed: `clSpriteAnime` keeps x at `+0x4C` and scale at `+0x40`,
//!   `clSpriteFont` keeps x at `+0x20`. **The two layouts differ**, which is
//!   why there is a hook per class here rather than one shared one.
//! - The transform that will eventually be applied is a *rigid translation*,
//!   not a scale: the same diff shows tatoohanz moved elements by exactly
//!   ±106 units -- the margin at 16:9 -- with the direction chosen per element.
//!   A scale would spread each element internally, taking a needle off its dial
//!   and opening gaps in per-glyph text.
//!
//! What this is here to find out:
//!
//! 1. Which consoles actually bracket sprite draws. A console whose wrapper
//!    sees **zero** sprites invalidates the whole approach for that element.
//! 2. Each console's x range, which is what decides its anchor -- a range near
//!    0 is left-anchored, one near 640 is right-anchored.
//! 3. Whether this all happens on one thread. It does not: the first run of
//!    this module reported `a sprite is drawn on another thread`, which is why
//!    `adm.rs` marshals sprite-adjacent calls onto the main thread in the first
//!    place. So `CURRENT_TAG` and the tallies, which are plain statics, can be
//!    read and written by more than one thread — tolerable here, where a
//!    mis-attributed sprite costs nothing and the ranges are what matter, but
//!    **not** something a transform built on this table may inherit.
//!
//! Anything with x well outside 0..640 is in some other space -- projected from
//! the world, or derived from the Lua screen size -- and must not get an anchor.

use crate::platform::call_game;
use crate::*;

/// The consoles that make up a race, each hooked so the sprites it draws can be
/// attributed to it. The index here is the tag, offset by one so that zero can
/// mean "no console was drawing".
const CONSOLES: [(&str, &str); 12] = [
	("_ZN14clMeterConsole4drawEv", "meter"),
	("_ZN14clMeterConsole5draw1Ev", "meter.draw1"),
	("_ZN14clMeterConsole5draw2Ev", "meter.draw2"),
	("_ZN12clMapConsole4drawEv", "map"),
	("_ZN13clTimeConsole4drawEv", "time"),
	("_ZN18clTotalTimeConsole4drawEv", "totaltime"),
	("_ZN20clSectionTimeConsole4drawEv", "sectiontime"),
	("_ZN14clShiftConsole4drawEv", "shift"),
	("_ZN18clRivalMarkConsole4drawEv", "rivalmark"),
	("_ZN18clRivalInfoConsole4drawEv", "rivalinfo"),
	("_ZN13clSignConsole4drawEv", "sign"),
	("_ZN17clStandardConsole4drawEv", "standard"),
];

#[derive(Clone, Copy)]
struct Tally {
	sprites: u32,
	fonts: u32,
	min_x: f32,
	max_x: f32,
}

impl Tally {
	const EMPTY: Self = Self {
		sprites: 0,
		fonts: 0,
		min_x: f32::INFINITY,
		max_x: f32::NEG_INFINITY,
	};
}

static mut TALLIES: [Tally; CONSOLES.len() + 1] = [Tally::EMPTY; CONSOLES.len() + 1];
/// 0 while no console is drawing, otherwise the console's index plus one.
static mut CURRENT_TAG: usize = 0;

static mut ORIGINAL_CONSOLES: [*const (); CONSOLES.len()] = [std::ptr::null(); CONSOLES.len()];
static mut ORIGINAL_SPRITE_ANIME: *const () = std::ptr::null();
static mut ORIGINAL_SPRITE_FONT: *const () = std::ptr::null();
static mut ORIGINAL_RACE_DRAW: *const () = std::ptr::null();

/// Reads the x this sprite is about to be drawn at and files it under whichever
/// console is drawing. `offset` differs per class; see the module comment.
unsafe fn record(this: *const c_void, offset: usize, font: bool) {
	if this.is_null() {
		return;
	}
	let x = (this as *const u8).add(offset).cast::<f32>().read();
	let tally = &mut TALLIES[CURRENT_TAG];
	if font {
		tally.fonts += 1;
	} else {
		tally.sprites += 1;
	}
	// NaN fails both comparisons and is simply not counted towards the range.
	if x < tally.min_x {
		tally.min_x = x;
	}
	if x > tally.max_x {
		tally.max_x = x;
	}
}

unsafe extern "C" fn sprite_anime_draw(this: *const c_void) -> *const c_void {
	report_thread_once(&mut SPRITE_THREAD_REPORTED, "a sprite");
	record(this, 0x4C, false);
	call_game(ORIGINAL_SPRITE_ANIME, (this,))
}

unsafe extern "C" fn sprite_font_draw(this: *const c_void) -> *const c_void {
	record(this, 0x20, true);
	call_game(ORIGINAL_SPRITE_FONT, (this,))
}

unsafe fn inside_console(index: usize, this: *const c_void) -> *const c_void {
	let previous = CURRENT_TAG;
	CURRENT_TAG = index + 1;
	let result = call_game(ORIGINAL_CONSOLES[index], (this,));
	CURRENT_TAG = previous;
	result
}

macro_rules! console_detours {
	($($detour:ident = $index:expr),* $(,)?) => {
		$(
			unsafe extern "C" fn $detour(this: *const c_void) -> *const c_void {
				inside_console($index, this)
			}
		)*
		const DETOURS: [unsafe extern "C" fn(*const c_void) -> *const c_void; CONSOLES.len()] =
			[$($detour,)*];
	};
}

console_detours!(
	console_0 = 0,
	console_1 = 1,
	console_2 = 2,
	console_3 = 3,
	console_4 = 4,
	console_5 = 5,
	console_6 = 6,
	console_7 = 7,
	console_8 = 8,
	console_9 = 9,
	console_10 = 10,
	console_11 = 11,
);

static mut RACE_THREAD_REPORTED: bool = false;
static mut SPRITE_THREAD_REPORTED: bool = false;

/// Says once, per site, whether this is the game's main thread. Two game calls,
/// so it is asked once and never again rather than per sprite.
unsafe fn report_thread_once(reported: &mut bool, what: &str) {
	if *reported {
		return;
	}
	*reported = true;
	platform::log(&format!(
		"hud: {what} is drawn on {}",
		if adm::is_main_thread() {
			"the main thread"
		} else {
			"another thread -- attribution below may be mixed"
		}
	));
}

static mut RACE_FRAMES: u32 = 0;
/// How often the table is printed, in race frames. 300 is five seconds at the
/// rate the game is held to.
const DUMP_EVERY: u32 = 300;

unsafe extern "C" fn race_draw(this: *const c_void) -> *const c_void {
	report_thread_once(&mut RACE_THREAD_REPORTED, "the race");
	// The consoles draw inside this call, so the tallies are complete only
	// after it returns.
	let result = call_game(ORIGINAL_RACE_DRAW, (this,));
	RACE_FRAMES += 1;
	if RACE_FRAMES % DUMP_EVERY == 0 {
		dump();
	}
	result
}

/// Prints what each console drew, then starts a fresh period so a later table
/// describes the race as it is then rather than everything since it began.
unsafe fn dump() {
	platform::log(&format!(
		"hud: ---- {DUMP_EVERY} race frames, sprite x by console ----"
	));
	for index in 0..TALLIES.len() {
		let tally = TALLIES[index];
		if tally.sprites == 0 && tally.fonts == 0 {
			continue;
		}
		let name = if index == 0 {
			"(no console drawing)"
		} else {
			CONSOLES[index - 1].1
		};
		platform::log(&format!(
			"hud: {name:<20} {:>6} sprites {:>5} fonts   x {:>9.2} .. {:<9.2}",
			tally.sprites, tally.fonts, tally.min_x, tally.max_x
		));
		TALLIES[index] = Tally::EMPTY;
	}
	// Named even when empty: a console that brackets no sprites at all is the
	// result that would invalidate the anchor approach for it, and silence
	// would hide that.
	let silent: Vec<&str> = (1..TALLIES.len())
		.filter(|index| TALLIES[*index].sprites == 0 && TALLIES[*index].fonts == 0)
		.map(|index| CONSOLES[index - 1].1)
		.collect();
	if !silent.is_empty() {
		platform::log(&format!("hud: drew nothing: {}", silent.join(", ")));
	}
}

pub unsafe fn init() {
	if !CONFIG.res_log {
		return;
	}
	for (index, (symbol, name)) in CONSOLES.iter().enumerate() {
		ORIGINAL_CONSOLES[index] = res::watch("hud", symbol, DETOURS[index] as *const (), name);
	}
	ORIGINAL_SPRITE_ANIME = res::watch(
		"hud",
		"_ZN13clSpriteAnime4drawEv",
		sprite_anime_draw as *const (),
		"clSpriteAnime::draw (x at +0x4C)",
	);
	ORIGINAL_SPRITE_FONT = res::watch(
		"hud",
		"_ZN12clSpriteFont4drawEv",
		sprite_font_draw as *const (),
		"clSpriteFont::draw (x at +0x20)",
	);
	ORIGINAL_RACE_DRAW = res::watch(
		"hud",
		"_ZN12clRaceThread4drawEv",
		race_draw as *const (),
		"clRaceThread::draw",
	);
}
