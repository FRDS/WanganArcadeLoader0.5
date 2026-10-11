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
//!   diffing tattoohanz's widescreen layouts against the dump's own shows x
//!   animated per keyframe, tens of thousands of values per file. Hooking
//!   `setPosition` would see almost none of it.
//! - The field offsets, read out of the dump's own 20-byte setters rather than
//!   assumed: `clSpriteAnime` keeps x at `+0x4C` and scale at `+0x40`,
//!   `clSpriteFont` keeps x at `+0x20`. **The two layouts differ**, which is
//!   why there is a hook per class here rather than one shared one.
//! - The transform that will eventually be applied is a *rigid translation*,
//!   not a scale: the same diff shows tattoohanz moved elements by exactly
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
	let previous = CURRENT_MARKER;
	let previous_widget = CURRENT_WIDGET;
	CURRENT_MARKER = MARKER_ANIME;
	CURRENT_WIDGET = widget_slot(anime_key(this));
	let result = call_game(ORIGINAL_SPRITE_ANIME, (this,));
	CURRENT_MARKER = previous;
	CURRENT_WIDGET = previous_widget;
	result
}

unsafe extern "C" fn sprite_font_draw(this: *const c_void) -> *const c_void {
	record(this, 0x20, true);
	let previous = CURRENT_MARKER;
	let previous_run = FONT_RUN;
	CURRENT_MARKER = MARKER_FONT;
	FONT_RUN = Run::EMPTY;
	let result = call_game(ORIGINAL_SPRITE_FONT, (this,));
	finish_string();
	FONT_RUN = previous_run;
	CURRENT_MARKER = previous;
	result
}

/// One string's quads, gathered across a single `clSpriteFont::draw` and
/// decided the moment it returns.
///
/// Fonts get no slot in the widget table. The first version of this keyed
/// them by object address, and the game churns through font objects so fast
/// that 1,548 of them were turned away from a 128-slot table in one race
/// while three live widgets sat in it. Identity was never the point for a
/// string: a string is one quad per glyph drawn inside one call, so the call
/// *is* the grouping, and the decision can be taken when it ends and the
/// quads thrown away. Nothing accumulates, so nothing can overflow.
#[derive(Clone, Copy)]
struct Run {
	quads: u32,
	min_x: f32,
	max_x: f32,
}

impl Run {
	const EMPTY: Self = Self {
		quads: 0,
		min_x: f32::INFINITY,
		max_x: f32::NEG_INFINITY,
	};
}

static mut FONT_RUN: Run = Run::EMPTY;
/// Strings decided `-bar`, `+bar` and `stay`, and the extremes over all of
/// them. A string that is torn by a per-quad rule shows up as the gap between
/// `FONT_SPAN` here and the per-quad `span` in the entry table.
static mut FONT_STRINGS: [u32; SIDE_NAMES.len()] = [0; SIDE_NAMES.len()];
static mut FONT_MIN: f32 = f32::INFINITY;
static mut FONT_MAX: f32 = f32::NEG_INFINITY;

const SIDE_LEFT: usize = 0;
const SIDE_RIGHT: usize = 1;
const SIDE_STAY: usize = 2;
const SIDE_WIDE: usize = 3;
const SIDE_OUT: usize = 4;
const SIDE_NAMES: [&str; 5] = ["-bar", "+bar", "stay", "wide", "out"];

/// The space everything is authored in.
const SPACE: f32 = 640.0;

/// The bar: how far an anchored widget moves, and equally how far the widened
/// box reaches past the authored edges. Being both at once is why it also
/// serves as the slack in `side` -- a quad anywhere inside the widened box is
/// still plausibly in the 2D layer, and one beyond it is not.
///
/// An arbitrary slack does not work. Tried at 24 units, it called
/// `anm 1432/2` -- a left-hand widget measured at −28.00..132.00 -- "not in
/// this space", when 28 units of overhang is nothing more than a sprite with
/// a soft edge. At the bar, every one of the 27 widgets the races produced
/// lands where inspection says it should.
fn bar(render_aspect: f32) -> f32 {
	SPACE / 2.0 * (render_aspect / (SPACE / 480.0) - 1.0)
}

/// `bar()` for the configured render, worked out once at startup.
static mut BAR: f32 = 0.0;

/// What a whole widget's extent says should be done with it.
///
/// The two verdicts past the obvious three exist because `stay` was giving
/// the right answer for the wrong reason, and anything built on that would
/// have inherited the mistake:
///
/// - **`wide`** is wider than the space it is authored in. Measured in a race:
///   `55032/390` spans −480..1120 and `6915144/1200` spans −444..750, against
///   a 640 space. Shifting full-width art does not help -- it just moves the
///   gap to the other edge -- so the roadmap's answer is to stretch those
///   quads, which is a different operation, not a smaller one.
/// - **`out`** reaches well beyond the space without being that wide, so it is
///   positioned in some other one -- projected from the world. Anchoring it
///   makes it jump every time it crosses the dead zone, which is exactly what
///   the roadmap says happens to the rival marker.
///
/// Neither can be told from the other by extent alone -- a world-projected
/// marker gets *wide* as it travels -- which is why the table prints this
/// period's extent beside the union. A static background reads the same in
/// both columns; something projected does not. `39904/259` read 224..416 in
/// one period and −268.80..908.80 in the next, and only the union caught it.
fn side(min_x: f32, max_x: f32, slack: f32) -> usize {
	if max_x - min_x > SPACE {
		SIDE_WIDE
	} else if min_x < -slack || max_x > SPACE + slack {
		SIDE_OUT
	} else if max_x < CENTRE - DEAD_ZONE {
		SIDE_LEFT
	} else if min_x > CENTRE + DEAD_ZONE {
		SIDE_RIGHT
	} else {
		SIDE_STAY
	}
}

unsafe fn finish_string() {
	if FONT_RUN.quads == 0 {
		return;
	}
	FONT_STRINGS[side(FONT_RUN.min_x, FONT_RUN.max_x, BAR)] += 1;
	if FONT_RUN.min_x < FONT_MIN {
		FONT_MIN = FONT_RUN.min_x;
	}
	if FONT_RUN.max_x > FONT_MAX {
		FONT_MAX = FONT_RUN.max_x;
	}
}

// ----------------------------------------------------------------- widgets
//
// The three lessons the roadmap paid for, as measurement rather than as a
// transform:
//
// 1. **Per widget, not per quad.** A string is one quad per glyph, so a
//    per-quad decision tears it apart at the dead-zone border. Every entry
//    submitted while a `clSpriteAnime` or `clSpriteFont` is drawing is filed
//    under that object.
// 2. **The whole widget's extent, not its first quad's.** The "till goal"
//    readout keeps digit slots either side of the centre and which exist
//    depends on the digit count, so a decision from one quad flips the whole
//    readout as the distance counts down.
// 3. **The union of every frame seen, never shrinking.** The tuning-point bar
//    starts as a sliver at the left of its frame and grows rightwards; judged
//    on its current extent it is pushed out of its own frame and snaps back
//    once it grows past the middle. So `min_x`/`max_x` here are never reset --
//    only the per-period quad count is.

const MAX_WIDGETS: usize = 128;
const NO_WIDGET: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Widget {
	used: bool,
	/// The `.anm`'s length and frame count. Animes only -- a string is
	/// grouped by its draw call instead; see `Run`.
	key: (u32, u32),
	/// Quads this period. The union below outlives it deliberately.
	quads: u32,
	/// The union over every frame this widget has ever been seen in.
	min_x: f32,
	max_x: f32,
	/// This period alone. Printed beside the union because the difference is
	/// what separates static art from something projected from the world --
	/// see `side`.
	now_min: f32,
	now_max: f32,
}

impl Widget {
	const EMPTY: Self = Self {
		used: false,
		key: (0, 0),
		quads: 0,
		min_x: f32::INFINITY,
		max_x: f32::NEG_INFINITY,
		now_min: f32::INFINITY,
		now_max: f32::NEG_INFINITY,
	};
}

static mut WIDGETS: [Widget; MAX_WIDGETS] = [Widget::EMPTY; MAX_WIDGETS];
/// Widgets that did not fit the table, which would make the rest misleading.
static mut WIDGETS_LOST: u32 = 0;
static mut CURRENT_WIDGET: usize = NO_WIDGET;

/// The `.anm` this object has loaded, as `(length, frame count)`, or `(0, 0)`
/// if it cannot be read safely.
///
/// That pair is the portable name, and an id is not: the roadmap records
/// `menu_g_finish_st_bar.anm` as 178 in WMMT3, 189 in 3DX and 199 in 3DX+,
/// and an image id is no better because the tuning bar's fill is swapped at
/// run time by `setImageReplace`. The buffer sits between `+0x18` (begin) and
/// `+0x14` (end), and its first dword is the frame count. Read off the object
/// the game already holds, so no `setAnime` hook -- which the roadmap records
/// as hanging 3DX.
///
/// Every step is guarded. This is a diagnostic, and a diagnostic must not be
/// the thing that takes the game down.
unsafe fn anime_key(this: *const c_void) -> (u32, u32) {
	if this.is_null() {
		return (0, 0);
	}
	let begin = (this as *const u8).add(0x18).cast::<*const u8>().read();
	let end = (this as *const u8).add(0x14).cast::<*const u8>().read();
	if begin.is_null() || end < begin {
		return (0, 0);
	}
	let length = end as usize - begin as usize;
	// A real `.anm` is a few bytes to a few megabytes and its buffer is
	// aligned. Anything else means this pair is not what we think it is, and
	// must not be dereferenced.
	if !(4..=0x0100_0000).contains(&length) || !(begin as usize).is_multiple_of(4) {
		return (0, 0);
	}
	(length as u32, begin.cast::<u32>().read())
}

/// The slot for this anime, adding one if it is new.
///
/// Only animes get a slot, and their keys are stable and few -- which is the
/// whole reason the table can be a fixed array that never evicts. Nothing
/// here is keyed by an address.
unsafe fn widget_slot(key: (u32, u32)) -> usize {
	let mut free = NO_WIDGET;
	for (index, widget) in WIDGETS.iter().enumerate() {
		if !widget.used {
			if free == NO_WIDGET {
				free = index;
			}
			continue;
		}
		if widget.key == key {
			return index;
		}
	}
	if free == NO_WIDGET {
		WIDGETS_LOST += 1;
		return NO_WIDGET;
	}
	WIDGETS[free] = Widget {
		used: true,
		key,
		..Widget::EMPTY
	};
	free
}

// ------------------------------------------------------------ sprite entries
//
// Where the x actually lives. The console route above answers its own question
// and the answer is no: five of the twelve consoles never draw, five submit a
// single object a frame at x = 0, and ten of every eleven sprites arrive under
// no console at all (measured 2026-10-11, 1280x720, eight tables in one race).
//
// `docs/ROADMAP.md` reached the same place from the other side: the race meter
// is not a `clSpriteAnime`, so `clMeterConsole::drawMeter`'s 17 raw entries a
// frame are invisible to the `+0x4C`/`+0x20` hooks whatever brackets them.
// Everything drawn, of every class, goes through `clSpriteManager::entry`, and
// by then the vertices are final -- so that is where a position can be both
// read and, eventually, changed.

/// The entry classes the dump defines, with the short name used in the table.
///
/// A live object's vptr is the vtable symbol **plus 8**: the Itanium ABI puts
/// offset-to-top and the RTTI pointer in front of the first virtual function.
/// The roadmap names four of these; the dump exports six.
const ENTRY_CLASSES: [(&str, &str); 6] = [
	("_ZTV17clSpriteRootEntry", "root"),
	("_ZTV13clSpriteEntry", "plain"),
	("_ZTV15clSpriteEntryUV", "plain.uv"),
	("_ZTV19clSpriteMatrixEntry", "matrix"),
	("_ZTV21clSpriteMatrixEntryUV", "matrix.uv"),
	("_ZTV22clSpriteRectIndexEntry", "rectindex"),
];

/// Index `ENTRY_CLASSES.len()` is "a vptr matching none of them", which is a
/// result in itself -- it would mean a seventh class, or that the `+8` is
/// wrong on this build.
const UNKNOWN_CLASS: usize = ENTRY_CLASSES.len();

const MARKER_BARE: usize = 0;
const MARKER_ANIME: usize = 1;
const MARKER_FONT: usize = 2;
const MARKER_NAMES: [&str; 3] = ["bare", "in anime", "in font"];

/// Whether a `clSpriteAnime::draw` or `clSpriteFont::draw` is on the stack.
/// The roadmap's plan demotes those two to marking which widget is being
/// drawn, so whether they actually bracket the entries is the thing to find
/// out before anything is built on them.
static mut CURRENT_MARKER: usize = MARKER_BARE;

static mut ENTRY_VPTRS: [*const (); ENTRY_CLASSES.len()] = [std::ptr::null(); ENTRY_CLASSES.len()];

/// The middle of the 640-space, and the roadmap's dead zone about it: a widget
/// entirely left of `CENTRE - DEAD_ZONE` moves `-bar`, one entirely right of
/// `CENTRE + DEAD_ZONE` moves `+bar`, and one spanning the middle stays put.
///
/// Counted here **per entry**, which is per quad -- and per quad is precisely
/// the decision the roadmap says tears a string apart at the dead-zone border,
/// since a string is one quad per glyph. So this is not the rule, it is the
/// shape of the data the rule will have to work on: it says how much of the
/// HUD wants to go left, how much right, and how much sits across the middle
/// and will need grouping before anything can be decided about it.
const CENTRE: f32 = 320.0;
const DEAD_ZONE: f32 = 8.0;

#[derive(Clone, Copy)]
struct EntryTally {
	count: u32,
	/// Entirely left of the dead zone, entirely right of it, and across it.
	left: u32,
	right: u32,
	span: u32,
	min_x: f32,
	max_x: f32,
}

impl EntryTally {
	const EMPTY: Self = Self {
		count: 0,
		left: 0,
		right: 0,
		span: 0,
		min_x: f32::INFINITY,
		max_x: f32::NEG_INFINITY,
	};
}

static mut ENTRIES: [[EntryTally; MARKER_NAMES.len()]; UNKNOWN_CLASS + 1] =
	[[EntryTally::EMPTY; MARKER_NAMES.len()]; UNKNOWN_CLASS + 1];

static mut ORIGINAL_ENTRY: *const () = std::ptr::null();
static mut ORIGINAL_ENTRY_AFTER: *const () = std::ptr::null();
static mut ENTRY_THREAD_REPORTED: bool = false;

/// The four finished vertex x floats, in 640x480 space.
const VERTEX_X: [usize; 4] = [0x08, 0x14, 0x20, 0x2c];

/// Files one submitted entry under its class and whatever is bracketing it.
///
/// Called **after** the original, because `entry()` runs `applyUser` when the
/// dirty byte at `+0x74` is not 1 and only then are the vertices the ones that
/// will be drawn. Reading first would see the previous frame's.
unsafe fn record_entry(entry: *const c_void) {
	if entry.is_null() {
		return;
	}
	let vptr = entry.cast::<*const ()>().read();
	let class = ENTRY_VPTRS
		.iter()
		.position(|known| *known == vptr)
		.unwrap_or(UNKNOWN_CLASS);
	// This entry's own extent first, so the side it falls on is decided from
	// the whole quad rather than from whichever vertex came last.
	let mut low = f32::INFINITY;
	let mut high = f32::NEG_INFINITY;
	for offset in VERTEX_X {
		let x = (entry as *const u8).add(offset).cast::<f32>().read();
		// NaN fails both, as in `record` above, and is simply not counted.
		if x < low {
			low = x;
		}
		if x > high {
			high = x;
		}
	}
	if low > high {
		// Every vertex was NaN. Counted, but it has no position to file.
		ENTRIES[class][CURRENT_MARKER].count += 1;
		return;
	}

	let tally = &mut ENTRIES[class][CURRENT_MARKER];
	tally.count += 1;
	if high < CENTRE - DEAD_ZONE {
		tally.left += 1;
	} else if low > CENTRE + DEAD_ZONE {
		tally.right += 1;
	} else {
		tally.span += 1;
	}
	if low < tally.min_x {
		tally.min_x = low;
	}
	if high > tally.max_x {
		tally.max_x = high;
	}

	// A string's quads go to the run its draw call will decide when it ends.
	// An anime's go to its slot, whose extent is the union over every frame
	// it has ever been seen in.
	if CURRENT_MARKER == MARKER_FONT {
		FONT_RUN.quads += 1;
		if low < FONT_RUN.min_x {
			FONT_RUN.min_x = low;
		}
		if high > FONT_RUN.max_x {
			FONT_RUN.max_x = high;
		}
	} else if CURRENT_WIDGET != NO_WIDGET {
		let widget = &mut WIDGETS[CURRENT_WIDGET];
		widget.quads += 1;
		if low < widget.min_x {
			widget.min_x = low;
		}
		if high > widget.max_x {
			widget.max_x = high;
		}
		if low < widget.now_min {
			widget.now_min = low;
		}
		if high > widget.now_max {
			widget.now_max = high;
		}
	}
}

unsafe extern "C" fn sprite_entry(this: *const c_void, entry: *const c_void) -> *const c_void {
	report_thread_once(&mut ENTRY_THREAD_REPORTED, "a sprite entry");
	let result = call_game(ORIGINAL_ENTRY, (this, entry));
	record_entry(entry);
	result
}

unsafe extern "C" fn sprite_entry_after(
	this: *const c_void,
	entry: *const c_void,
) -> *const c_void {
	let result = call_game(ORIGINAL_ENTRY_AFTER, (this, entry));
	record_entry(entry);
	result
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

static mut FRAMES: u32 = 0;
/// How often the table is printed, in presented frames. 300 is five seconds at
/// the rate the game is held to.
const DUMP_EVERY: u32 = 300;

/// Prints the table every `DUMP_EVERY` frames. Called from `adm.rs`'s swap,
/// which is the one path certain to run.
///
/// **It is driven from the frame path and not from the race, because
/// `clRaceThread::draw` is never called.** Measured at 1280x720 on 2026-10-11,
/// in a real race on the Daikoku Line with the HUD plainly on screen: the
/// sprite hook fired -- `hud: a sprite is drawn on another thread` is in that
/// log -- and `race_draw` below did not run once. The dump used to hang off
/// it, so the tallies filled up for an entire race and nothing was ever
/// printed. `clRaceThread::draw` looks vestigial: the class also exports
/// `run(void*)`, `race()`, `yield()` and `starting()`, and the per-frame work
/// reaches the consoles without going through it.
///
/// A dump happens only when some console drew. The attract loop draws sprites
/// in quantity with every console silent, and an empty table every five
/// seconds for as long as a cabinet idles is exactly the per-frame logging
/// `report.rs` exists to prevent.
pub unsafe fn tick() {
	if !CONFIG.res_log {
		return;
	}
	FRAMES += 1;
	if FRAMES % DUMP_EVERY != 0 {
		return;
	}
	// Tag 0 is sprites drawn under no console at all. It is worth reporting
	// inside a table but must never be the reason for one.
	let drew = (1..TALLIES.len()).any(|index| {
		let tally = TALLIES[index];
		tally.sprites > 0 || tally.fonts > 0
	});
	if drew {
		dump();
	} else {
		// The period ends either way. Without this the first table of a race
		// would carry every entry submitted since boot -- the whole attract
		// loop -- and its counts would mean nothing.
		TALLIES = [Tally::EMPTY; CONSOLES.len() + 1];
		ENTRIES = [[EntryTally::EMPTY; MARKER_NAMES.len()]; UNKNOWN_CLASS + 1];
		// Only the counts: a widget's extent is the union over every frame it
		// has been seen in, and an idle period is not a reason to forget it.
		for widget in WIDGETS.iter_mut() {
			widget.quads = 0;
		}
	}
}

unsafe extern "C" fn race_draw(this: *const c_void) -> *const c_void {
	// Kept hooked, though nothing has ever reached it, so that the log says so
	// on the day something does. See `tick` for the measurement.
	report_thread_once(&mut RACE_THREAD_REPORTED, "the race");
	call_game(ORIGINAL_RACE_DRAW, (this,))
}

/// Prints what each console drew, then starts a fresh period so a later table
/// describes the race as it is then rather than everything since it began.
unsafe fn dump() {
	platform::log(&format!(
		"hud: ---- {DUMP_EVERY} frames, sprite x by console ----"
	));
	// Taken before anything is printed, because the loop below clears each
	// tally as it reports it -- so a list built afterwards finds every console
	// empty and names all twelve, including the ones just printed above it.
	let silent: Vec<&str> = (1..TALLIES.len())
		.filter(|index| TALLIES[*index].sprites == 0 && TALLIES[*index].fonts == 0)
		.map(|index| CONSOLES[index - 1].1)
		.collect();
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
	// would hide that. Measured on 2026-10-11, five of the twelve are in here
	// through a whole race -- meter, meter.draw1, map, sectiontime and
	// rivalinfo -- while the minimap and the rival panel are plainly on
	// screen, so their draw() is as vestigial as clRaceThread's.
	if !silent.is_empty() {
		platform::log(&format!("hud: drew nothing: {}", silent.join(", ")));
	}

	// The entry table. Unlike the one above, every quad the engine draws is in
	// here, of every class, with the x it will actually be drawn at.
	let mut headed = false;
	for class in 0..ENTRIES.len() {
		for marker in 0..MARKER_NAMES.len() {
			let tally = ENTRIES[class][marker];
			if tally.count == 0 {
				continue;
			}
			if !headed {
				platform::log("hud: ---- entries by class, and what brackets them ----");
				headed = true;
			}
			let name = if class == UNKNOWN_CLASS {
				"(unknown)"
			} else {
				ENTRY_CLASSES[class].1
			};
			platform::log(&format!(
				"hud: {name:<10} {:<9} {:>6}  x {:>8.2} .. {:<8.2}  left {:>5}  right {:>5}  span {:>5}",
				MARKER_NAMES[marker],
				tally.count,
				tally.min_x,
				tally.max_x,
				tally.left,
				tally.right,
				tally.span
			));
			ENTRIES[class][marker] = EntryTally::EMPTY;
		}
	}

	// The widget table: what the transform will actually decide over. The
	// extent is every frame this widget has been seen in, so a bar that grows
	// or a readout that gains digits has already widened it.
	let mut headed = false;
	for widget in WIDGETS.iter_mut() {
		if !widget.used || widget.quads == 0 {
			continue;
		}
		if !headed {
			platform::log("hud: ---- widgets, union extent over every frame seen ----");
			headed = true;
		}
		// The roadmap's rule, applied to the whole widget rather than a quad,
		// and to the union rather than to this period.
		let decision = SIDE_NAMES[side(widget.min_x, widget.max_x, BAR)];
		platform::log(&format!(
			"hud: anm {:<7}/{:<5} {:>6} quads  union {:>9.2} ..{:>9.2}  \
			 now {:>9.2} ..{:>9.2}  {decision}",
			widget.key.0,
			widget.key.1,
			widget.quads,
			widget.min_x,
			widget.max_x,
			widget.now_min,
			widget.now_max
		));
		widget.quads = 0;
		widget.now_min = f32::INFINITY;
		widget.now_max = f32::NEG_INFINITY;
	}
	if WIDGETS_LOST > 0 {
		platform::log(&format!(
			"hud: {WIDGETS_LOST} animes did not fit the table of {MAX_WIDGETS} -- the rows above are a subset"
		));
	}

	// Strings, decided one draw call at a time and counted rather than kept.
	let strings: u32 = FONT_STRINGS.iter().sum();
	if strings > 0 {
		// The counts are this period's; the range is every string ever seen,
		// and is labelled so because the two are not the same window.
		let counts: Vec<String> = SIDE_NAMES
			.iter()
			.enumerate()
			.map(|(index, name)| format!("{name} {:>5}", FONT_STRINGS[index]))
			.collect();
		platform::log(&format!(
			"hud: strings  {strings:>6} decided  ever {FONT_MIN:>9.2} ..{FONT_MAX:>9.2}  {}",
			counts.join("  ")
		));
		FONT_STRINGS = [0; SIDE_NAMES.len()];
	}
}

pub unsafe fn init() {
	if !CONFIG.res_log {
		return;
	}
	BAR = bar(CONFIG.width as f32 / CONFIG.height as f32);
	platform::log(&format!("hud: the bar is {BAR:.2} units at this render"));
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

	// The vtables, so a submitted entry can be told apart by its vptr. A class
	// whose symbol is missing simply never matches, and its entries land under
	// "(unknown)" rather than being mis-filed as another class.
	let mut found = 0;
	for (index, (symbol, name)) in ENTRY_CLASSES.iter().enumerate() {
		let vtable = platform::get_symbol(symbol);
		if vtable.is_null() {
			platform::log(&format!("hud: no vtable for {name}, {symbol} is missing"));
			continue;
		}
		ENTRY_VPTRS[index] = (vtable as *const u8).add(8) as *const ();
		found += 1;
	}
	platform::log(&format!(
		"hud: {found} of {} entry vtables resolved",
		ENTRY_CLASSES.len()
	));

	ORIGINAL_ENTRY = res::watch(
		"hud",
		"_ZN15clSpriteManager5entryER17clSpriteRootEntry",
		sprite_entry as *const (),
		"clSpriteManager::entry",
	);
	ORIGINAL_ENTRY_AFTER = res::watch(
		"hud",
		"_ZN15clSpriteManager10entryAfterER17clSpriteRootEntry",
		sprite_entry_after as *const (),
		"clSpriteManager::entryAfter",
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The bar at the three aspects that matter, against the figure three
	/// independent routes agree on.
	#[test]
	fn the_bar_is_what_three_sources_say() {
		assert!((bar(16.0 / 9.0) - 106.666_67).abs() < 0.01);
		// 4:3 is what the dump was built for, so there is nothing to move.
		assert!(bar(4.0 / 3.0).abs() < 0.001);
		assert!((bar(16.0 / 10.0) - 64.0).abs() < 0.01);
	}

	/// Every widget the races of 2026-10-11 produced, at 1280x720, with the
	/// union extent the loader reported for it.
	///
	/// These are measurements, not examples. A change to the thresholds has
	/// to face the HUD the game actually draws rather than invented numbers,
	/// which is the whole reason the first slack of 24 units did not survive
	/// contact with `1432/2`.
	#[test]
	fn classifies_every_widget_a_race_produced() {
		let slack = bar(16.0 / 9.0);
		let side = |min, max| side(min, max, slack);

		// Left-hand cluster: the lap/section readouts and the gear display.
		assert_eq!(side(32.0, 309.0), SIDE_LEFT); // anm 71176/200
		assert_eq!(side(27.0, 124.0), SIDE_LEFT); // anm 3672/18
		assert_eq!(side(15.0, 143.0), SIDE_LEFT); // anm 1824/3
		assert_eq!(side(14.0, 118.0), SIDE_LEFT); // anm 2408/4
		assert_eq!(side(10.0, 195.0), SIDE_LEFT); // anm 1584/5
		// Pokes 28 units off the left edge and is still a left-hand widget.
		assert_eq!(side(-28.0, 132.0), SIDE_LEFT); // anm 1432/2

		// Right-hand cluster: the speedometer and its companions.
		assert_eq!(side(511.5, 659.5), SIDE_RIGHT); // anm 19832/70
		assert_eq!(side(536.0, 640.0), SIDE_RIGHT); // anm 216/2
		assert_eq!(side(436.0, 640.0), SIDE_RIGHT); // anm 3552/3
		assert_eq!(side(522.0, 626.0), SIDE_RIGHT); // anm 1304/2

		// Genuinely across the middle, and inside the space.
		assert_eq!(side(189.4, 540.0), SIDE_STAY); // anm 98152/660
		assert_eq!(side(304.0, 432.0), SIDE_STAY); // anm 14352/273
		assert_eq!(side(192.0, 448.0), SIDE_STAY); // anm 14944/259
		assert_eq!(side(289.0, 353.0), SIDE_STAY); // anm 9352/200
		assert_eq!(side(-10.01, 539.12), SIDE_STAY); // anm 126984/200
		assert_eq!(side(170.0, 682.0), SIDE_STAY); // anm 336/1
		assert_eq!(side(209.0, 721.0), SIDE_STAY); // anm 7992/30

		// Wider than the space: shifting these only moves the gap.
		assert_eq!(side(0.0, 1024.0), SIDE_WIDE); // anm 272/1
		assert_eq!(side(-192.0, 1024.0), SIDE_WIDE); // anm 144/1
		assert_eq!(side(-192.0, 832.0), SIDE_WIDE); // anm 13256/200
		assert_eq!(side(-480.0, 1120.0), SIDE_WIDE); // anm 55032/390
		assert_eq!(side(-444.0, 750.0), SIDE_WIDE); // anm 6915144/1200
		assert_eq!(side(-724.97, 857.45), SIDE_WIDE); // anm 161032/600
		assert_eq!(side(-128.65, 640.5), SIDE_WIDE); // anm 97672/200
		assert_eq!(side(-41.71, 695.0), SIDE_WIDE); // anm 291912/400

		// Projected from the world. `39904/259` read 224..416 in one period
		// and this in the next, which is why the union decides and not the
		// period.
		assert_eq!(side(-268.8, 908.8), SIDE_WIDE); // anm 39904/259
		assert_eq!(side(-1129.08, 947.61), SIDE_WIDE); // anm 3176/140

		// Narrow but wholly beyond the widened box: the one shape that is
		// `out` rather than `wide`, and the reason that verdict exists.
		assert_eq!(side(800.0, 1000.0), SIDE_OUT);
		assert_eq!(side(-400.0, -200.0), SIDE_OUT);
	}

	/// A widget exactly on the dead zone's edge does not move, and one a
	/// whisker clear of it does.
	#[test]
	fn the_dead_zone_is_inclusive() {
		let slack = bar(16.0 / 9.0);
		assert_eq!(side(100.0, CENTRE - DEAD_ZONE, slack), SIDE_STAY);
		assert_eq!(side(100.0, CENTRE - DEAD_ZONE - 0.1, slack), SIDE_LEFT);
		assert_eq!(side(CENTRE + DEAD_ZONE, 500.0, slack), SIDE_STAY);
		assert_eq!(side(CENTRE + DEAD_ZONE + 0.1, 500.0, slack), SIDE_RIGHT);
	}
}
