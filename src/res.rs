//! Custom resolution: the screen size the engine reads out of Lua, the minimap
//! viewport, the perspective FOV -- and, behind `res_log`, an account of every
//! projection the engine builds, because the engine reports none itself.
//!
//! The 2D layer is the reason `widen` exists. Measured on hardware at two
//! aspects, the engine asks for the same two boxes either way --
//! `[0, 639, 447, 0, -100, 100]` and `[0, 640, 480, 0, -1, 1]` -- while the
//! viewport follows the render size. So the 2D layer is laid out in 640-space
//! and mapped onto whatever the viewport is: at 1280x720 that is x2.0 against
//! y1.5, a third again too wide. Widening the box to the render aspect makes
//! the mapping isotropic again, which is one hook rather than the dozens a
//! per-sprite correction needs.

use crate::platform::call_game;
use crate::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// What everything in the dump was laid out for.
const ORIGINAL_ASPECT: f32 = 640.0 / 480.0;

/// The horizontal range a screen-space orthographic box needs so that a render
/// at `render_aspect` maps it isotropically, or `None` to leave the box alone.
///
/// The widening is symmetric about the box's own centre, so the 2D layer stays
/// centred and the room this buys appears at both edges -- showing the world
/// the perspective hook has already widened, where letterboxing showed black.
/// The vertical range is never touched: scaling both would undo itself.
///
/// **Most of this function is about what not to touch.**
/// `makeOrthographicProjection` is a general matrix utility, so the same call
/// builds world-space projections -- shadows among them -- that widening would
/// corrupt. A screen-space box is recognised by being anchored at the top-left
/// with y growing downward, and by having roughly the aspect the dump was laid
/// out at: the engine's own two boxes are 640x480 (1.333) and 640x448 (1.429),
/// and a square render-to-texture pass or a world box looks like neither.
/// The run that measured those two reached the attract screens, not a race, so
/// a box this rejects is a thing `res_log` is for.
fn widen(left: f32, right: f32, bottom: f32, top: f32, render_aspect: f32) -> Option<(f32, f32)> {
	// 4:3 is what the dump was built for, so there is nothing to correct --
	// and this keeps the configuration proven on three machines untouched.
	if render_aspect == ORIGINAL_ASPECT {
		return None;
	}
	// Anchored at the top-left, y downward: a screen box, not a world box.
	if left != 0.0 || top != 0.0 || right <= 0.0 || bottom <= 0.0 {
		return None;
	}
	// Laid out at something near 4:3, rather than square or panoramic.
	if !(1.2..=1.5).contains(&(right / bottom)) {
		return None;
	}
	let half = right / 2.0;
	let factor = render_aspect / ORIGINAL_ASPECT;
	Some((half - half * factor, half + half * factor))
}

// ------------------------------------------------------------------ reporting

/// Argument sets already reported, so a call the engine makes every frame --
/// or every sprite -- is logged once. The same shape, and the same reason, as
/// `files.rs::first_failure`: an early `gl_debug` build without deduplication
/// produced a 70 MB log in a single run.
static REPORTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
const MAX_REPORTED: usize = 64;

fn log_once(line: String) {
	let Ok(mut reported) = REPORTED.lock() else {
		return;
	};
	let reported = reported.get_or_insert_with(HashSet::new);
	if reported.len() >= MAX_REPORTED || !reported.insert(line.clone()) {
		return;
	}
	platform::log(&line);
}

/// How often the engine asks for an orthographic projection. Once a frame or
/// once a sprite is the difference between a hook that can stay installed and
/// one that cannot, so count it rather than guess.
static ORTHO_CALLS: AtomicUsize = AtomicUsize::new(0);

fn count_ortho_call() {
	let count = ORTHO_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
	if matches!(count, 1 | 100 | 10_000 | 1_000_000) {
		platform::log(&format!("res: orthographic projection call #{count}"));
	}
}

/// `N` floats from a pointer the game owns, or `None` if it is null.
/// Diagnostics must not be the thing that takes the game down.
unsafe fn floats<const N: usize>(from: *const c_float) -> Option<[f32; N]> {
	(!from.is_null()).then(|| std::array::from_fn(|i| from.add(i).read()))
}

unsafe fn doubles<const N: usize>(from: *const c_double) -> Option<[f64; N]> {
	(!from.is_null()).then(|| std::array::from_fn(|i| from.add(i).read()))
}

// ------------------------------------------------------- the screen size Lua sees

static mut LUA_GETGLOBAL: *const () = std::ptr::null();
static mut LUA_SETGLOBAL: *const () = std::ptr::null();
static mut LUA_PUSHNUMBER: *const () = std::ptr::null();

unsafe fn set_global_number(state: *const c_void, global: *const c_char, value: c_double) {
	call_game::<_, ()>(LUA_PUSHNUMBER, (state, value));
	call_game::<_, ()>(LUA_SETGLOBAL, (state, global));
}
unsafe extern "C" fn lua_getglobal(state: *const c_void, global: *const c_char) -> c_int {
	let str = CStr::from_ptr(global).to_str().unwrap();
	let replaced = match str {
		"SCREEN_XSIZE" => Some(CONFIG.width as c_double),
		"SCREEN_YSIZE" => Some(CONFIG.height as c_double),
		"MINIMAP_DISP_X" => Some((CONFIG.width as c_double * 0.0265625).round()),
		"MINIMAP_DISP_Y" => Some((CONFIG.height as c_double * 0.2364).round()),
		_ => None,
	};
	if let Some(value) = replaced {
		set_global_number(state, global, value);
		if CONFIG.res_log {
			log_once(format!("res: lua {str} = {value}"));
		}
	}

	call_game(LUA_GETGLOBAL, (state, global))
}

// ------------------------------------------------------------------- viewport

static mut ORIGINAL_SET_VIEWPORT: *const () = std::ptr::null();
unsafe extern "C" fn set_viewport(
	this: *const c_void,
	a1: c_int,
	a2: c_int,
	width: c_int,
	height: c_int,
	a5: c_float,
	a6: c_float,
) -> *const c_void {
	if width == 88 && height == 82 {
		let width = CONFIG.width as f32 * 0.1375;
		let height = CONFIG.height as f32 * 0.17;
		if CONFIG.res_log {
			log_once(format!(
				"res: setViewport {a1},{a2} 88x82 (the minimap) -> {}x{}",
				width as c_int, height as c_int
			));
		}
		return call_game(
			ORIGINAL_SET_VIEWPORT,
			(this, a1, a2, width as c_int, height as c_int, a5, a6),
		);
	}
	if CONFIG.res_log {
		log_once(format!(
			"res: setViewport {a1},{a2} {width}x{height} {a5} {a6}"
		));
	}
	call_game(ORIGINAL_SET_VIEWPORT, (this, a1, a2, width, height, a5, a6))
}

// ---------------------------------------------------------------- projections

static mut ORIGINAL_MAKE_PERSPECTIVE: *const () = std::ptr::null();
unsafe extern "C" fn make_perspective(
	this: *const c_void,
	fov: c_float,
	a2: c_float,
	aspect_ratio: c_float,
	a4: c_float,
	a5: c_float,
) {
	let width = CONFIG.width as f32;
	let height = CONFIG.height as f32;
	let original_aspect_ratio = ORIGINAL_ASPECT;
	let new_aspect_ratio = if aspect_ratio == original_aspect_ratio {
		width / height
	} else {
		aspect_ratio
	};
	let new_fov = fov / original_aspect_ratio * new_aspect_ratio;
	if CONFIG.res_log {
		log_once(format!(
			"res: perspective fov {fov} -> {new_fov}, aspect {aspect_ratio} -> {new_aspect_ratio} \
			 (near {a4} far {a5}, a2 {a2})"
		));
	}
	call_game::<_, ()>(
		ORIGINAL_MAKE_PERSPECTIVE,
		(this, new_fov, a2, new_aspect_ratio, a4, a5),
	)
}

// The four orthographic overloads the dump exports. Every one is a pass-through
// that only reports: which of them the 2D layer actually uses, and with what
// box, is the measurement. The arguments are logged positionally rather than
// named l/r/b/t/n/f, because their order is an assumption and the numbers are
// what will confirm it.
//
// Each returns a pointer rather than nothing: the mangled name does not carry
// the return type, and these may well return the matrix they filled. Passing
// eax through is right either way -- a void function leaves it undefined and
// its caller ignores it.

static mut ORIGINAL_ORTHO_F: *const () = std::ptr::null();
unsafe extern "C" fn ortho_f(
	this: *const c_void,
	a1: c_float,
	a2: c_float,
	a3: c_float,
	a4: c_float,
	a5: c_float,
	a6: c_float,
) -> *const c_void {
	let render_aspect = CONFIG.width as f32 / CONFIG.height as f32;
	let widened = CONFIG
		.widescreen_hud
		.then(|| widen(a1, a2, a3, a4, render_aspect))
		.flatten();
	let (left, right) = widened.unwrap_or((a1, a2));
	if CONFIG.res_log {
		count_ortho_call();
		match widened {
			Some(_) => log_once(format!(
				"res: ortho f: [{a1}, {a2}, {a3}, {a4}, {a5}, {a6}] -> left {left} right {right}"
			)),
			None => log_once(format!(
				"res: ortho f: [{a1}, {a2}, {a3}, {a4}, {a5}, {a6}] left alone"
			)),
		}
	}
	call_game(ORIGINAL_ORTHO_F, (this, left, right, a3, a4, a5, a6))
}

// The three overloads below are installed only when `res_log` is on: nothing
// has ever seen the engine call them, and the measurement that showed that is
// the only reason to keep watching.
static mut ORIGINAL_ORTHO_F_PLANE: *const () = std::ptr::null();
unsafe extern "C" fn ortho_f_plane(
	this: *const c_void,
	plane: *const c_float,
	point: *const c_float,
) -> *const c_void {
	count_ortho_call();
	let plane_values = floats::<4>(plane);
	let point_values = floats::<3>(point);
	log_once(format!(
		"res: ortho f plane: plane {plane_values:?} point {point_values:?}"
	));
	call_game(ORIGINAL_ORTHO_F_PLANE, (this, plane, point))
}

static mut ORIGINAL_ORTHO_D: *const () = std::ptr::null();
unsafe extern "C" fn ortho_d(
	this: *const c_void,
	a1: c_double,
	a2: c_double,
	a3: c_double,
	a4: c_double,
	a5: c_double,
	a6: c_double,
) -> *const c_void {
	count_ortho_call();
	log_once(format!(
		"res: ortho d: [{a1}, {a2}, {a3}, {a4}, {a5}, {a6}]"
	));
	call_game(ORIGINAL_ORTHO_D, (this, a1, a2, a3, a4, a5, a6))
}

static mut ORIGINAL_ORTHO_D_PLANE: *const () = std::ptr::null();
unsafe extern "C" fn ortho_d_plane(
	this: *const c_void,
	plane: *const c_double,
	point: *const c_double,
) -> *const c_void {
	count_ortho_call();
	let plane_values = doubles::<4>(plane);
	let point_values = doubles::<3>(point);
	log_once(format!(
		"res: ortho d plane: plane {plane_values:?} point {point_values:?}"
	));
	call_game(ORIGINAL_ORTHO_D_PLANE, (this, plane, point))
}

/// Hooks `symbol` and says whether this dump has it. Which overloads exist is
/// part of what the log is being read for, and a symbol that is missing or
/// refused leaves a null trampoline, so it has to be said rather than assumed.
pub(crate) unsafe fn watch(
	prefix: &str,
	symbol: &str,
	func: *const (),
	what: &str,
) -> *const () {
	let original = hook::hook_symbol(symbol, func);
	if original.is_null() {
		platform::log(&format!("{prefix}: cannot watch {what}, {symbol} is missing"));
	} else {
		platform::log(&format!("{prefix}: watching {what}"));
	}
	original
}

pub unsafe fn init() {
	LUA_GETGLOBAL = hook::hook_symbol("lua_getglobal", lua_getglobal as *const ());
	LUA_SETGLOBAL = hook::get_symbol("lua_setglobal");
	LUA_PUSHNUMBER = hook::get_symbol("lua_pushnumber");

	ORIGINAL_SET_VIEWPORT = hook::hook_symbol(
		"_ZN3Gap3Gfx19igAGLEVisualContext11setViewportEiiiiff",
		set_viewport as *const (),
	);
	ORIGINAL_MAKE_PERSPECTIVE = hook::hook_symbol(
		"_ZN3Gap4Math11igMatrix44f32makePerspectiveProjectionRadiansEfffff",
		make_perspective as *const (),
	);

	// The float overload is the one the engine uses -- measured, not assumed --
	// so it is installed whenever it has something to do. It is reached a few
	// times a frame, not once a sprite, which the call counter established.
	if CONFIG.widescreen_hud || CONFIG.res_log {
		ORIGINAL_ORTHO_F = watch(
			"res",
			"_ZN3Gap4Math11igMatrix44f26makeOrthographicProjectionEffffff",
			ortho_f as *const (),
			"igMatrix44f::makeOrthographicProjection(float x6)",
		);
	}

	// The rest have never been seen to fire. They stay watchable, because
	// "nothing fired" is only worth trusting if something was listening.
	if !CONFIG.res_log {
		return;
	}
	ORIGINAL_ORTHO_F_PLANE = watch(
		"res",
		"_ZN3Gap4Math11igMatrix44f26makeOrthographicProjectionEPKNS0_7igPlaneERKNS0_7igVec3fE",
		ortho_f_plane as *const (),
		"igMatrix44f::makeOrthographicProjection(igPlane, igVec3f)",
	);
	ORIGINAL_ORTHO_D = watch(
		"res",
		"_ZN3Gap4Math11igMatrix44d26makeOrthographicProjectionEdddddd",
		ortho_d as *const (),
		"igMatrix44d::makeOrthographicProjection(double x6)",
	);
	ORIGINAL_ORTHO_D_PLANE = watch(
		"res",
		"_ZN3Gap4Math11igMatrix44d26makeOrthographicProjectionEPKNS0_7igPlaneERKNS0_7igVec3dE",
		ortho_d_plane as *const (),
		"igMatrix44d::makeOrthographicProjection(igPlane, igVec3d)",
	);
}

#[cfg(test)]
mod tests {
	use super::{widen, ORIGINAL_ASPECT};

	/// The two shapes a 4:3 layout actually meets.
	const SIXTEEN_NINE: f32 = 1280.0 / 720.0;
	const SIXTEEN_TEN: f32 = 1280.0 / 800.0;

	fn width((left, right): (f32, f32)) -> f32 {
		right - left
	}

	fn centre((left, right): (f32, f32)) -> f32 {
		(left + right) / 2.0
	}

	#[test]
	fn a_4_3_render_needs_no_correction() {
		// The configuration proven on three machines must come out untouched.
		assert_eq!(widen(0.0, 640.0, 480.0, 0.0, ORIGINAL_ASPECT), None);
	}

	#[test]
	fn widens_the_screen_box_to_the_render_aspect() {
		// The box the engine asked for on hardware, at both aspects.
		let wide = widen(0.0, 640.0, 480.0, 0.0, SIXTEEN_NINE).expect("the screen box");
		// Isotropic: the box's own aspect now matches what it is drawn into.
		assert!((width(wide) / 480.0 - SIXTEEN_NINE).abs() < 0.001, "{wide:?}");
		// ...and the layer has not moved.
		assert!((centre(wide) - 320.0).abs() < 0.001, "{wide:?}");
	}

	#[test]
	fn scales_by_the_aspect_and_nothing_else() {
		let wide = widen(0.0, 640.0, 480.0, 0.0, SIXTEEN_TEN).expect("the screen box");
		assert!((width(wide) - 480.0 * SIXTEEN_TEN).abs() < 0.001, "{wide:?}");
	}

	#[test]
	fn keeps_the_engines_own_anisotropy_in_its_other_box() {
		// 640x448 is the engine's second box, and 1.429 is not 4:3. Removing
		// the render aspect's distortion must not also "fix" that, or the box
		// stops being the one the dump was laid out against.
		let wide = widen(0.0, 639.0, 447.0, 0.0, SIXTEEN_NINE).expect("the second box");
		let before = 639.0 / 447.0;
		let after = width(wide) / 447.0;
		assert!(
			(after / before - SIXTEEN_NINE / ORIGINAL_ASPECT).abs() < 0.001,
			"{wide:?}"
		);
	}

	#[test]
	fn leaves_everything_that_is_not_the_screen_alone() {
		// This same call builds world-space projections -- shadows among them
		// -- and widening one of those corrupts it.
		assert_eq!(widen(-50.0, 50.0, -50.0, 50.0, SIXTEEN_NINE), None);
		// y the other way up, or anchored somewhere else.
		assert_eq!(widen(0.0, 640.0, 0.0, 480.0, SIXTEEN_NINE), None);
		assert_eq!(widen(10.0, 650.0, 480.0, 0.0, SIXTEEN_NINE), None);
		// Square, as a render-to-texture pass would be.
		assert_eq!(widen(0.0, 256.0, 256.0, 0.0, SIXTEEN_NINE), None);
		// Panoramic, and degenerate.
		assert_eq!(widen(0.0, 1024.0, 256.0, 0.0, SIXTEEN_NINE), None);
		assert_eq!(widen(0.0, 0.0, 0.0, 0.0, SIXTEEN_NINE), None);
	}
}
