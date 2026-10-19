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
	CURRENT_WIDGET = widget_slot(KIND_ANIME, anime_key(this));
	let result = call_game(ORIGINAL_SPRITE_ANIME, (this,));
	CURRENT_MARKER = previous;
	CURRENT_WIDGET = previous_widget;
	result
}

unsafe extern "C" fn sprite_font_draw(this: *const c_void) -> *const c_void {
	record(this, 0x20, true);
	let previous = CURRENT_MARKER;
	let previous_run = FONT_RUN;
	let previous_widget = CURRENT_WIDGET;
	CURRENT_MARKER = MARKER_FONT;
	FONT_RUN = Run::EMPTY;
	// The run decides this call for the summary; the slot carries the union
	// across frames, which is what the move needs.
	CURRENT_WIDGET = widget_slot(KIND_FONT, font_key(this));
	let result = call_game(ORIGINAL_SPRITE_FONT, (this,));
	finish_string(this);
	CURRENT_WIDGET = previous_widget;
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
const SIDE_NONE: usize = 5;
const SIDE_NAMES: [&str; 6] = ["-bar", "+bar", "stay", "wide", "out", "none"];

/// The narrowest extent that counts as a position. One unit is 1.5 pixels at
/// 1280, so nothing visible is excluded.
const MIN_WIDTH: f32 = 1.0;

/// The longest a widget is made to wait for its animation to come round
/// before it may be moved. Five seconds at the rate the game is held to.
const SETTLE_CAP: u32 = 300;

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

/// The least slack `side` will work with, whatever the bar is.
///
/// The bar doubles as the slack because the widened box reaches exactly that
/// far past the authored edges -- but at 4:3 the bar is zero, and then a
/// widget like `anm 1432/2` at -30.00..130.00 is "outside the space" over
/// nothing more than a soft sprite edge. Measured at 1280x960. Nothing moves
/// at 4:3 so it changed no pixels, but the same collapse would happen at any
/// aspect near it.
const MIN_SLACK: f32 = 32.0;

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
	// No width is no position, whatever the arithmetic below would make of
	// it. `anm 4392/60` sits at 0.00 .. 0.00 and the first build of the
	// transform shoved it a bar to the left on the strength of `0 < 320`.
	// It is a root or a container -- the same population the console table
	// kept reporting as `x 0.00 .. 0.00` -- and nothing visible is zero
	// quads wide.
	let width = max_x - min_x;
	if width.is_nan() || width < MIN_WIDTH {
		SIDE_NONE
	} else if width > SPACE {
		SIDE_WIDE
	} else if min_x < -slack.max(MIN_SLACK) || max_x > SPACE + slack.max(MIN_SLACK) {
		SIDE_OUT
	} else if max_x < CENTRE - DEAD_ZONE {
		SIDE_LEFT
	} else if min_x > CENTRE + DEAD_ZONE {
		SIDE_RIGHT
	} else {
		SIDE_STAY
	}
}

unsafe fn finish_string(this: *const c_void) {
	if FONT_RUN.quads == 0 {
		return;
	}
	FONT_STRINGS[side(FONT_RUN.min_x, FONT_RUN.max_x, BAR)] += 1;
	// Once per slot: the object's own x and y beside the extent its quads
	// came out at. These disagreed badly -- `txt@320,384` drew over 0..158
	// against an origin of 320 -- which is what led to the stale-vertex
	// finding, since `submit` now forces `applyUser` before reading. Kept as
	// the check that it stays fixed: origin and extent should now agree.
	if CURRENT_WIDGET != NO_WIDGET && !this.is_null() && !WIDGETS[CURRENT_WIDGET].announced {
		WIDGETS[CURRENT_WIDGET].announced = true;
		let x = (this as *const u8).add(0x20).cast::<f32>().read();
		let y = (this as *const u8).add(0x24).cast::<f32>().read();
		platform::log(&format!(
			"hud: string at +0x20 ({x:.2}, {y:.2}) drew {} quads over x {:.2} .. {:.2}, \
			 so origin+extent is {:.2} .. {:.2}",
			FONT_RUN.quads,
			FONT_RUN.min_x,
			FONT_RUN.max_x,
			x + FONT_RUN.min_x,
			x + FONT_RUN.max_x
		));
	}
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

const MAX_WIDGETS: usize = 512;
const NO_WIDGET: usize = usize::MAX;

/// An anime, named by its `.anm`; a string, named by where it starts; or a
/// console, named by its index.
///
/// The console kind exists because **the race meter is not a
/// `clSpriteAnime`** -- the roadmap says so and hardware agrees: measured on
/// 2026-10-17, the dial sat 123 units from the right edge at 16:9 against 19
/// at 4:3, having never moved at all. Its entries arrive inside
/// `clMeterConsole::draw2` with no anime and no font around them, so they had
/// no widget, and `shift` returned zero before it considered anything. Same
/// principle as a string: the call that brackets them is the grouping.
const KIND_ANIME: u8 = 0;
const KIND_FONT: u8 = 1;
const KIND_CLUSTER: u8 = 2;

/// How close two quads must be to belong to the same thing.
///
/// Taking a whole console as one widget does not work: `clMeterConsole`'s
/// `draw1` submits 7 raw quads a frame spanning **-64.00 .. 685.00**, which
/// is wider than the 640 space it lives in, so it classified `wide` and
/// nothing moved. It is a container for several unrelated pieces -- a dial
/// on the right, something else out at the left -- not a single widget.
///
/// So raw entries are gathered into runs of touching quads instead. A dial
/// is a disc of quads that overlap or abut, and the gear display twenty units
/// away is a different run. Eight units is wide enough to bridge the seams
/// inside one piece of art and far too narrow to bridge the gaps between
/// pieces.
const CLUSTER_GAP: f32 = 8.0;

/// How coarsely a string's origin is rounded to key its slot. Four units
/// absorbs the jitter of a readout whose origin is derived from its own
/// width, without merging two pieces of text that sit side by side.
const SLOT: f32 = 4.0;

#[derive(Clone, Copy)]
struct Widget {
	used: bool,
	/// What kind of thing this widget is, which decides how `key` reads and
	/// how long it has to settle.
	kind: u8,
	/// An anime's `.anm` length and frame count, a string's rounded origin,
	/// or a console's index. Never an address: keying strings by address is what
	/// overflowed a 128-slot table 1,548 times in one race, because the game
	/// churns through font objects. An origin is a place on the screen, and
	/// the same piece of text comes back to it frame after frame.
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
	/// Whether this widget has finished at least one frame. Until it has, its
	/// union is a part-built thing -- the quads of the frame in progress --
	/// and a verdict taken from it would move the first half of a widget and
	/// not the second.
	settled: bool,
	/// Frames this widget has drawn in, and the last one it drew in, so the
	/// count is of frames and not of quads.
	frames_seen: u32,
	last_frame: u32,
	/// Whether the log has already said this one is being moved. Said once
	/// per widget rather than through `report.rs`, because this sits on the
	/// per-entry path and a flag costs nothing where building the string to
	/// deduplicate on would cost something sixty-five times a frame.
	announced: bool,
}

impl Widget {
	const EMPTY: Self = Self {
		used: false,
		kind: KIND_ANIME,
		key: (0, 0),
		quads: 0,
		min_x: f32::INFINITY,
		max_x: f32::NEG_INFINITY,
		now_min: f32::INFINITY,
		now_max: f32::NEG_INFINITY,
		settled: false,
		frames_seen: 0,
		last_frame: 0,
		announced: false,
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
unsafe fn widget_slot(kind: u8, key: (u32, u32)) -> usize {
	let mut free = NO_WIDGET;
	for (index, widget) in WIDGETS.iter().enumerate() {
		if !widget.used {
			if free == NO_WIDGET {
				free = index;
			}
			continue;
		}
		if widget.kind == kind && widget.key == key {
			return index;
		}
	}
	if free == NO_WIDGET {
		WIDGETS_LOST += 1;
		return NO_WIDGET;
	}
	WIDGETS[free] = Widget {
		used: true,
		kind,
		key,
		..Widget::EMPTY
	};
	free
}

/// How a widget is written in the log.
unsafe fn widget_name(widget: &Widget) -> String {
	match widget.kind {
		KIND_FONT => format!(
			"txt@{},{}",
			widget.key.0 as i32 * SLOT as i32,
			widget.key.1 as i32 * SLOT as i32
		),
		KIND_CLUSTER => format!(
			"{}#{}",
			CONSOLES[widget.key.0 as usize].1,
			widget.key.1
		),
		_ => format!("anm {}/{}", widget.key.0, widget.key.1),
	}
}

/// The slot a string belongs to: where on the screen it starts.
///
/// A string has no `.anm` to be named by and its object address churns, but
/// it is drawn at the same place frame after frame -- so the place is the
/// identity. `clSpriteFont` keeps x at `+0x20` and y at `+0x24`, taken from
/// the dump's own 20-byte setters.
unsafe fn font_key(this: *const c_void) -> (u32, u32) {
	if this.is_null() {
		return (0, 0);
	}
	let x = (this as *const u8).add(0x20).cast::<f32>().read();
	let y = (this as *const u8).add(0x24).cast::<f32>().read();
	if !x.is_finite() || !y.is_finite() {
		return (0, 0);
	}
	(
		(x / SLOT).round() as i32 as u32,
		(y / SLOT).round() as i32 as u32,
	)
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

/// Classes already reported as drawing a quad with no width.
static mut DEGENERATE_SEEN: [bool; UNKNOWN_CLASS + 1] = [false; UNKNOWN_CLASS + 1];

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
/// This entry's own extent, or `None` if no vertex was a number.
///
/// Taken from the whole quad rather than from whichever vertex came last, so
/// the side it falls on is decided from all four.
unsafe fn entry_extent(entry: *const c_void) -> Option<(f32, f32)> {
	let mut low = f32::INFINITY;
	let mut high = f32::NEG_INFINITY;
	for offset in VERTEX_X {
		let x = (entry as *const u8).add(offset).cast::<f32>().read();
		// NaN fails both comparisons and is simply not counted.
		if x < low {
			low = x;
		}
		if x > high {
			high = x;
		}
	}
	(low <= high).then_some((low, high))
}

unsafe fn record_entry(entry: *const c_void, low: f32, high: f32, owner: usize) {
	let vptr = entry.cast::<*const ()>().read();
	let class = ENTRY_VPTRS
		.iter()
		.position(|known| *known == vptr)
		.unwrap_or(UNKNOWN_CLASS);

	// A quad with no width, after `applyUser` has run, means the vertices are
	// not where this class keeps its position. The roadmap puts a 2x3 affine
	// at `+0x7c` with its translation at `+0x8c`/`+0x90`; if that holds a
	// sensible x while the vertices are zero, then for this class the
	// vertices are the wrong thing to read *and* the wrong thing to write,
	// and the offset belongs in the affine. Said once per class, so it costs
	// nothing on a path that runs sixty-five times a frame.
	if low >= high && !DEGENERATE_SEEN[class] {
		DEGENERATE_SEEN[class] = true;
		let tx = (entry as *const u8).add(0x8c).cast::<f32>().read();
		let ty = (entry as *const u8).add(0x90).cast::<f32>().read();
		let name = if class == UNKNOWN_CLASS {
			"(unknown)"
		} else {
			ENTRY_CLASSES[class].1
		};
		platform::log(&format!(
			"hud: {name} drew a quad with no width -- vertices x all {low:.2}, \
			 but the affine at +0x8c holds ({tx:.2}, {ty:.2})"
		));
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
	}
	if owner != NO_WIDGET {
		let widget = &mut WIDGETS[owner];
		widget.quads += 1;
		// Frames, not quads: a widget drawing ten quads a frame is no nearer
		// having shown its whole extent than one drawing a single quad.
		if widget.last_frame != FRAMES {
			widget.last_frame = FRAMES;
			widget.frames_seen += 1;
		}
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
	submit(ORIGINAL_ENTRY, this, entry)
}

unsafe extern "C" fn sprite_entry_after(
	this: *const c_void,
	entry: *const c_void,
) -> *const c_void {
	submit(ORIGINAL_ENTRY_AFTER, this, entry)
}

// ------------------------------------------------------------------ the move

/// Whether the anchor is on, settled once at startup.
static mut ANCHOR: bool = false;

/// Whether widgets are being moved, for `res.rs` -- the minimap's viewport is
/// placed by a different mechanism and has to follow the frame that `entry`
/// moves, or the art and its box come apart.
pub fn anchoring() -> bool {
	unsafe { ANCHOR }
}

/// Measures one entry, moves it if its widget says so, and submits it.
///
/// The order is the whole trick, and the roadmap paid for it: `entry()` calls
/// `applyUser` whenever the dirty byte at `+0x74` is not 1, and *then* copies
/// twelve floats into the draw buffer. So anything written into the vertices
/// before `entry()` runs is thrown away by the `applyUser` it goes on to make.
/// Calling `applyUser` here, marking the entry clean and only then moving the
/// finished vertices is the sequence that survives, and it works for every
/// entry class -- including the plain ones whose coordinates are absolute and
/// the matrix ones rebuilt from the affine at `+0x7c`.
///
/// The measurement is taken from the finished vertices *before* they are
/// moved. Reading them after would feed the shift back into the union and
/// walk each widget off the screen a bar per frame.
unsafe fn submit(
	original: *const (),
	this: *const c_void,
	entry: *const c_void,
) -> *const c_void {
	if entry.is_null() {
		return call_game(original, (this, entry));
	}

	// **Finish the vertices first, always.** `entry()` runs `applyUser` only
	// when the dirty byte at `+0x74` is not 1, so for an entry that is
	// already clean the floats are whatever was last left in them. Measured
	// 2026-10-17: every string in a menu read `9 quads over x 0.00 .. 0.00`
	// against an origin of 48 or 320, while the same strings read 158..290
	// in the moving path, which forces `applyUser` on its way through. The
	// two paths were measuring different things and the difference looked
	// like a property of the strings.
	finish_vertices(entry);
	let Some((low, high)) = entry_extent(entry) else {
		return call_game(original, (this, entry));
	};

	// Who owns this quad. An anime or a string that is drawing claims its
	// own; a raw entry submitted straight by a console belongs to the run of
	// touching quads it falls in, because a console is a container for
	// several unrelated pieces rather than one widget.
	let owner = if CURRENT_WIDGET != NO_WIDGET {
		CURRENT_WIDGET
	} else if CURRENT_CONSOLE != NO_CONSOLE {
		cluster_slot(CURRENT_CONSOLE, low, high)
	} else {
		NO_WIDGET
	};

	// Measured before it is moved. Reading afterwards would feed the shift
	// back into the union and walk the widget off the screen a bar a frame.
	record_entry(entry, low, high, owner);
	let by = shift(owner);
	if by != 0.0 {
		offset_x(entry, by);
	}
	call_game(original, (this, entry))
}

/// Runs `applyUser` -- vtable slot 2, the call at `+0x8` -- and marks the
/// entry clean so `entry()` will not run it again over the top.
unsafe fn finish_vertices(entry: *const c_void) {
	let vptr = entry.cast::<*const *const ()>().read();
	if vptr.is_null() {
		return;
	}
	let apply_user = vptr.add(2).read();
	if apply_user.is_null() {
		return;
	}
	call_game::<_, ()>(apply_user, (entry,));
	(entry as *mut u8).add(0x74).write(1);
}

/// Moves the four finished vertex x floats, which is the whole transform.
unsafe fn offset_x(entry: *const c_void, by: f32) {
	for offset in VERTEX_X {
		let at = (entry as *mut u8).add(offset).cast::<f32>();
		at.write(at.read() + by);
	}
}

/// How far the widget now drawing should move.
///
/// Zero unless the verdict is a clean `-bar` or `+bar`. `stay` is across the
/// middle, `wide` wants stretching rather than shifting, and `out` is not in
/// this space at all -- the rival marker jumps every time it crosses the dead
/// zone if you anchor it. Zero too for a string, which has no settled union
/// to decide from, and for a widget that has not finished a frame yet.
unsafe fn shift(owner: usize) -> f32 {
	if !ANCHOR || owner == NO_WIDGET || !in_race() {
		return 0.0;
	}
	let widget = WIDGETS[owner];
	if !widget.settled || widget.min_x > widget.max_x {
		return 0.0;
	}
	// Wait until the animation has had the chance to play through once,
	// because until then the union is not the widget's extent but only as
	// much of it as has been reached. `anm 10440/160` was measured at
	// -84.30..462.30 in one run and 81.01..296.99 early in another: the
	// first spans the middle and stays put, the second reads as a left-hand
	// widget and moves. Acting on the second and then meeting the first is a
	// widget that jumps a bar sideways mid-race. The frame count is already
	// half of the fingerprint, so what to wait for is known rather than
	// guessed; the cap keeps a 1200-frame animation from waiting 20 seconds,
	// and nothing that long has ever come out as anything but `wide`.
	// **Strings are measured but not moved.** Tried on hardware on
	// 2026-10-17 and pulled the same run.
	//
	// Most of them are fine: `txt@160,128` drew over 158..290 against an
	// origin of 160, `txt@80,80` over 78..268 against 80. Those are honest
	// absolute positions in the 640-space and their `-bar` verdicts are
	// right. But a minority disagree with their own origin -- `txt@48,40`
	// drew over 0..78 against an origin of 48, and `txt@320,352` over 0..62
	// against 320, which is wrong at both ends by about the origin. Either
	// those quads are local to their object, or a degenerate frame at zero
	// has poisoned a union that by design never shrinks.
	//
	// Those outliers were all seen on menu screens, and `in_race` above now
	// keeps the anchor away from those entirely. In a race the strings are
	// right -- the player's name sits where it should beside the speed
	// readout, confirmed on hardware. So strings move again, inside a race
	// only, and `finish_string` keeps logging the origin beside the extent
	// so the disagreement can still be chased down.
	//
	// A string has no animation to come round, so one frame of it is enough;
	// an anime waits for its own frame count.
	// Only an anime has an animation to come round; a string and a console
	// are complete as soon as one frame of them has been seen.
	let needed = if widget.kind == KIND_ANIME {
		widget.key.1.clamp(1, SETTLE_CAP)
	} else {
		1
	};
	if widget.frames_seen < needed {
		return 0.0;
	}
	let by = match side(widget.min_x, widget.max_x, BAR) {
		SIDE_LEFT => -BAR,
		SIDE_RIGHT => BAR,
		_ => 0.0,
	};
	// Said once per widget, so a run that reaches no race still shows in the
	// log whether the transform fired at all and on what.
	// Behind `res_log`: with the anchor on and nothing being diagnosed, a run
	// should be quiet. This is a measurement of what moved, not news.
	if CONFIG.res_log && by != 0.0 && !widget.announced {
		WIDGETS[owner].announced = true;
		let name = widget_name(&widget);
		platform::log(&format!(
			"hud: moving {name} by {by:+.2}, it sits at {:.2} .. {:.2}",
			widget.min_x, widget.max_x
		));
	}
	by
}

unsafe fn inside_console(index: usize, this: *const c_void) -> *const c_void {
	let previous = CURRENT_TAG;
	let previous_widget = CURRENT_WIDGET;
	CURRENT_TAG = index + 1;
	CONSOLE_FRAME = FRAMES;
	// Raw entries drawn straight by the console -- the meter's among them --
	// are gathered into runs of touching quads within it. An anime or a
	// string inside the console claims its own entries instead, and puts
	// this back when it returns.
	let previous_console = CURRENT_CONSOLE;
	CURRENT_CONSOLE = index;
	CURRENT_WIDGET = NO_WIDGET;
	let result = call_game(ORIGINAL_CONSOLES[index], (this,));
	CURRENT_CONSOLE = previous_console;
	CURRENT_WIDGET = previous_widget;
	CURRENT_TAG = previous;
	result
}

/// The console whose own drawing is in progress, if any.
static mut CURRENT_CONSOLE: usize = NO_CONSOLE;
const NO_CONSOLE: usize = usize::MAX;

/// The run of touching quads a raw entry belongs to, adding one if it starts
/// a new run.
///
/// Runs are kept per console and never merged. Two that grow until they touch
/// stay separate, which is the conservative way round: a run that should have
/// been one piece moves in two halves that were already going the same way,
/// whereas merging eagerly risks chaining the whole console into one `wide`
/// blob again -- which is the failure this exists to fix.
unsafe fn cluster_slot(console: usize, low: f32, high: f32) -> usize {
	let mut free = NO_WIDGET;
	let mut ordinal = 0;
	for (index, widget) in WIDGETS.iter().enumerate() {
		if !widget.used {
			if free == NO_WIDGET {
				free = index;
			}
			continue;
		}
		if widget.kind != KIND_CLUSTER || widget.key.0 as usize != console {
			continue;
		}
		ordinal += 1;
		if low <= widget.max_x + CLUSTER_GAP && high >= widget.min_x - CLUSTER_GAP {
			return index;
		}
	}
	if free == NO_WIDGET {
		WIDGETS_LOST += 1;
		return NO_WIDGET;
	}
	WIDGETS[free] = Widget {
		used: true,
		kind: KIND_CLUSTER,
		key: (console as u32, ordinal),
		..Widget::EMPTY
	};
	free
}

/// The last frame a race console drew anything.
static mut CONSOLE_FRAME: u32 = 0;
/// How long after one the anchor keeps working, so it does not flicker off
/// between two consoles in the same frame or across a brief gap.
const RACE_GRACE: u32 = 30;

/// Whether a race is on screen, which is the only place the anchor is safe.
///
/// **Menus must not be anchored.** Measured on 2026-10-17, on the name entry
/// screen: its character grid is laid out as many separate widgets, so
/// `ABCDE` and `FGHIJ` went `-bar`, `LMNO` and `PQRST` went `+bar`, `K`
/// straddled the middle and stayed, and the row came apart with 213 units of
/// gap either side of the `K`. "Enter" was pushed into "lower case" the same
/// way. Deciding each widget on its own is right for a race HUD, whose
/// clusters really are independent and already sit near the edges, and wrong
/// for a layout whose pieces only mean anything side by side.
///
/// That is also what the roadmap was telling us: tattoohanz did not move the
/// in-race HUD, and 36 of his 39 replacement layouts are menus -- he used
/// assets there because menus need per-element control, and his `hud_43` box
/// mode is the designed answer for this screen. Grouping a whole menu layout
/// is a separate piece of work; until then the anchor stays where it is
/// demonstrably right.
unsafe fn in_race() -> bool {
	CONSOLE_FRAME != 0 && FRAMES.saturating_sub(CONSOLE_FRAME) < RACE_GRACE
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
	if !CONFIG.res_log && !ANCHOR {
		return;
	}
	// A widget that drew anything this frame has a complete extent from the
	// next frame on, and can be decided about. Before that its union holds
	// only the quads submitted so far, and a verdict taken from it would move
	// the first half of a widget and leave the rest behind.
	FRAMES += 1;
	for widget in WIDGETS.iter_mut() {
		if widget.quads > 0 {
			widget.settled = true;
		}
	}
	if !CONFIG.res_log {
		return;
	}
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
		let name = widget_name(widget);
		platform::log(&format!(
			"hud: {name:<17} {:>6} quads  union {:>9.2} ..{:>9.2}  \
			 now {:>9.2} ..{:>9.2}  {decision}",
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

/// Like `res::watch`, but quiet about success unless something is being
/// diagnosed.
///
/// `widescreen_anchor` is a feature, not an investigation: with it on and
/// `res_log` off, a boot should not print twenty lines about hooks that
/// installed exactly as expected. A hook that *fails* is always worth saying,
/// because every verdict downstream of it silently changes.
unsafe fn watch(symbol: &str, func: *const (), what: &str) -> *const () {
	let original = hook::hook_symbol(symbol, func);
	if original.is_null() {
		platform::log(&format!("hud: cannot watch {what}, {symbol} is missing"));
	} else if CONFIG.res_log {
		platform::log(&format!("hud: watching {what}"));
	}
	original
}

pub unsafe fn init() {
	// `widescreen_anchor` moves widgets, `res_log` only watches them. Either
	// needs the sprite and entry hooks; only the measurement needs the
	// console wrappers, which found nothing and are kept for what they say
	// about the consoles rather than for what they attribute.
	ANCHOR = CONFIG.widescreen_anchor && CONFIG.widescreen_hud;
	if CONFIG.widescreen_anchor && !CONFIG.widescreen_hud {
		platform::log(
			"hud: widescreen_anchor does nothing while widescreen_hud is off -- \
			 the distance to move is the margin that one opens up",
		);
	}
	if !CONFIG.res_log && !ANCHOR {
		return;
	}
	BAR = bar(CONFIG.width as f32 / CONFIG.height as f32);
	platform::log(&format!(
		"hud: the bar is {BAR:.2} units at this render, {}",
		if ANCHOR { "anchoring" } else { "measuring only" }
	));
	// The console wrappers attribute almost nothing, which is what they were
	// installed to find out -- but they are also the only thing that says a
	// race is on screen rather than a menu, so the anchor needs them too.
	for (index, (symbol, name)) in CONSOLES.iter().enumerate() {
		ORIGINAL_CONSOLES[index] = watch(symbol, DETOURS[index] as *const (), name);
	}
	if CONFIG.res_log {
		ORIGINAL_RACE_DRAW = watch(
			"_ZN12clRaceThread4drawEv",
			race_draw as *const (),
			"clRaceThread::draw",
		);
	}
	ORIGINAL_SPRITE_ANIME = watch(
				"_ZN13clSpriteAnime4drawEv",
		sprite_anime_draw as *const (),
		"clSpriteAnime::draw (x at +0x4C)",
	);
	ORIGINAL_SPRITE_FONT = watch(
				"_ZN12clSpriteFont4drawEv",
		sprite_font_draw as *const (),
		"clSpriteFont::draw (x at +0x20)",
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
	if CONFIG.res_log {
		platform::log(&format!(
			"hud: {found} of {} entry vtables resolved",
			ENTRY_CLASSES.len()
		));
	}

	ORIGINAL_ENTRY = watch(
				"_ZN15clSpriteManager5entryER17clSpriteRootEntry",
		sprite_entry as *const (),
		"clSpriteManager::entry",
	);
	ORIGINAL_ENTRY_AFTER = watch(
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

	/// A widget with no width has no position, and `0.00 .. 0.00` must not
	/// read as "left of centre". `anm 4392/60` is a real one, caught being
	/// moved a bar to the left on the first run of the transform.
	#[test]
	fn no_width_is_not_a_position() {
		let slack = bar(16.0 / 9.0);
		assert_eq!(side(0.0, 0.0, slack), SIDE_NONE); // anm 4392/60
		assert_eq!(side(400.0, 400.0, slack), SIDE_NONE);
		assert_eq!(side(0.0, 0.5, slack), SIDE_NONE);
		// An empty union, which is what a widget looks like before its first
		// quad arrives, must not come out as a direction either.
		assert_eq!(side(f32::INFINITY, f32::NEG_INFINITY, slack), SIDE_NONE);
		// A unit wide is thin but real, and still has a side.
		assert_eq!(side(100.0, 101.0, slack), SIDE_LEFT);
	}

	/// At 4:3 the bar is zero, so the slack derived from it is too, and
	/// `anm 1432/2` at -30.00..130.00 came out `out` over a soft edge.
	/// Measured at 1280x960.
	#[test]
	fn the_slack_does_not_collapse_at_four_three() {
		assert_eq!(side(-30.0, 130.0, bar(4.0 / 3.0)), SIDE_LEFT);
		assert_eq!(side(-30.0, 130.0, bar(16.0 / 9.0)), SIDE_LEFT);
		// Still outside when it is genuinely outside, at any aspect.
		assert_eq!(side(-200.0, -100.0, bar(4.0 / 3.0)), SIDE_OUT);
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
