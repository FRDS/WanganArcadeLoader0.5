use crate::*;

static mut LUA_GETGLOBAL: Option<extern "C" fn(*const c_void, *const c_char) -> c_int> = None;
static mut LUA_SETGLOBAL: Option<extern "C" fn(*const c_void, *const c_char)> = None;
static mut LUA_PUSHNUMBER: Option<extern "C" fn(*const c_void, c_double)> = None;
unsafe extern "C" fn lua_getglobal(state: *const c_void, global: *const c_char) -> c_int {
	let str = CStr::from_ptr(global).to_str().unwrap();
	match str {
		"SCREEN_XSIZE" => {
			LUA_PUSHNUMBER.unwrap()(state, CONFIG.width as c_double);
			LUA_SETGLOBAL.unwrap()(state, global);
		}
		"SCREEN_YSIZE" => {
			LUA_PUSHNUMBER.unwrap()(state, CONFIG.height as c_double);
			LUA_SETGLOBAL.unwrap()(state, global);
		}
		"MINIMAP_DISP_X" => {
			LUA_PUSHNUMBER.unwrap()(state, (CONFIG.width as c_double * 0.0265625).round());
			LUA_SETGLOBAL.unwrap()(state, global);
		}
		"MINIMAP_DISP_Y" => {
			LUA_PUSHNUMBER.unwrap()(state, (CONFIG.height as c_double * 0.2364).round());
			LUA_SETGLOBAL.unwrap()(state, global);
		}
		_ => {}
	};

	LUA_GETGLOBAL.unwrap()(state, global)
}

static mut ORIGINAL_SET_VIEWPORT: Option<
	extern "C" fn(*const c_void, c_int, c_int, c_int, c_int, c_float, c_float) -> *const c_void,
> = None;
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
		return ORIGINAL_SET_VIEWPORT.unwrap()(
			this,
			a1,
			a2,
			width as c_int,
			height as c_int,
			a5,
			a6,
		);
	}
	ORIGINAL_SET_VIEWPORT.unwrap()(this, a1, a2, width, height, a5, a6)
}

static mut ORIGINAL_MAKE_PERSPECTIVE: Option<
	extern "C" fn(*const c_void, c_float, c_float, c_float, c_float, c_float),
> = None;
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
	ORIGINAL_MAKE_PERSPECTIVE.unwrap()(this, fov, a2, aspect_ratio, a4, a5)
}

pub unsafe fn init() {
	LUA_GETGLOBAL = Some(transmute(hook::hook_symbol(
		"lua_getglobal",
		lua_getglobal as *const (),
	)));
	LUA_SETGLOBAL = Some(transmute(hook::get_symbol("lua_setglobal")));
	LUA_PUSHNUMBER = Some(transmute(hook::get_symbol("lua_pushnumber")));

	ORIGINAL_SET_VIEWPORT = Some(transmute(hook::hook_symbol(
		"_ZN3Gap3Gfx19igAGLEVisualContext11setViewportEiiiiff",
		set_viewport as *const (),
	)));
	ORIGINAL_MAKE_PERSPECTIVE = Some(transmute(hook::hook_symbol(
		"_ZN3Gap4Math11igMatrix44f32makePerspectiveProjectionRadiansEfffff",
		make_perspective as *const (),
	)));
}
