use crate::platform::call_game;
use crate::*;

static mut LUA_GETGLOBAL: *const () = std::ptr::null();
static mut LUA_SETGLOBAL: *const () = std::ptr::null();
static mut LUA_PUSHNUMBER: *const () = std::ptr::null();

unsafe fn set_global_number(state: *const c_void, global: *const c_char, value: c_double) {
	call_game::<_, ()>(LUA_PUSHNUMBER, (state, value));
	call_game::<_, ()>(LUA_SETGLOBAL, (state, global));
}
unsafe extern "C" fn lua_getglobal(state: *const c_void, global: *const c_char) -> c_int {
	let str = CStr::from_ptr(global).to_str().unwrap();
	match str {
		"SCREEN_XSIZE" => {
			set_global_number(state, global, CONFIG.width as c_double);
		}
		"SCREEN_YSIZE" => {
			set_global_number(state, global, CONFIG.height as c_double);
		}
		"MINIMAP_DISP_X" => {
			set_global_number(
				state,
				global,
				(CONFIG.width as c_double * 0.0265625).round(),
			);
		}
		"MINIMAP_DISP_Y" => {
			set_global_number(state, global, (CONFIG.height as c_double * 0.2364).round());
		}
		_ => {}
	};

	call_game(LUA_GETGLOBAL, (state, global))
}

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
		return call_game(
			ORIGINAL_SET_VIEWPORT,
			(this, a1, a2, width as c_int, height as c_int, a5, a6),
		);
	}
	call_game(ORIGINAL_SET_VIEWPORT, (this, a1, a2, width, height, a5, a6))
}

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
	let original_aspect_ratio = 640.0 / 480.0;
	let aspect_ratio = if aspect_ratio == original_aspect_ratio {
		width / height
	} else {
		aspect_ratio
	};
	let fov = fov / original_aspect_ratio * aspect_ratio;
	call_game::<_, ()>(
		ORIGINAL_MAKE_PERSPECTIVE,
		(this, fov, a2, aspect_ratio, a4, a5),
	)
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
}
