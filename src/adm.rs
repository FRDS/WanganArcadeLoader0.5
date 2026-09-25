use crate::platform::call_game;
use crate::*;
use glfw::*;
use std::mem::forget;

extern "C" fn adm_version() -> *const c_char {
	let cstr = CString::new("WanganArcadeLoader 0.1").unwrap();
	let ptr = cstr.as_ptr();
	forget(cstr);
	ptr
}

/// The game window, for reading keyboard state in poll.rs.
pub static mut GLFW_WINDOW: *mut glfw::ffi::GLFWwindow = std::ptr::null_mut();

#[allow(non_snake_case)]
#[repr(C)]
#[derive(Default)]
struct AdmChooseMode {
	ident: [u8; 4], // MOCF
	unk_0x04: u32,
	unk_0x08: u32,
	unk_0x0C: u32,
	unk_0x10: u32,
	unk_0x14: u32,
	width: u32,
	height: u32,
	refresh: u32,
}

#[repr(C)]
struct AdmWindow {
	ident: [u8; 4], // WNDW
	glfw: Glfw,
	window: PWindow,
	fbo: u32,
}

extern "C" fn adm_config() -> *const *const AdmChooseMode {
	let adm = unsafe {
		AdmChooseMode {
			ident: [b'M', b'O', b'C', b'F'],
			width: CONFIG.width,
			height: CONFIG.height,
			refresh: 60,
			..Default::default()
		}
	};
	Box::leak(Box::new(Box::leak(Box::new(adm)) as *const AdmChooseMode))
}

extern "C" fn adm_fb_config() -> *const u8 {
	Box::leak(Box::new(0))
}

unsafe extern "C" fn adm_window() -> *mut AdmWindow {
	let mut glfw = glfw::init(glfw::fail_on_errors).unwrap();
	glfw.window_hint(WindowHint::Resizable(false)); // Force floating on tiling window managers
	let (mut window, _) = glfw.with_primary_monitor(|glfw, m| {
		// Fullscreen is borderless at the monitor's current mode, so the display
		// mode never changes; adm_swap_buffers scales the game's frame to fit.
		let fullscreen = if CONFIG.fullscreen {
			m.and_then(|m| m.get_video_mode().map(|mode| (m, mode)))
		} else {
			None
		};
		match fullscreen {
			Some((m, mode)) => {
				glfw.window_hint(WindowHint::RedBits(Some(mode.red_bits)));
				glfw.window_hint(WindowHint::GreenBits(Some(mode.green_bits)));
				glfw.window_hint(WindowHint::BlueBits(Some(mode.blue_bits)));
				glfw.window_hint(WindowHint::RefreshRate(Some(mode.refresh_rate)));
				glfw.create_window(
					mode.width,
					mode.height,
					"WanganArcadeLoader",
					WindowMode::FullScreen(m),
				)
			}
			None => glfw.create_window(
				CONFIG.width,
				CONFIG.height,
				"WanganArcadeLoader",
				WindowMode::Windowed,
			),
		}
		.unwrap()
	});
	GLFW_WINDOW = window.window_ptr();
	window.make_current();
	window.set_resizable(true);
	glfw.set_swap_interval(SwapInterval::Sync(1));

	opengl::load_gl_funcs(&glfw);
	gl::load_with(|s| glfw.get_proc_address_raw(s));

	let mut fbo = 0;
	let mut texture = 0;
	gl::GenFramebuffers(1, &mut fbo);
	gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);

	gl::GenTextures(1, &mut texture);
	gl::BindTexture(gl::TEXTURE_2D, texture);
	gl::TexImage2D(
		gl::TEXTURE_2D,
		0,
		gl::RGB as i32,
		CONFIG.width as i32,
		CONFIG.height as i32,
		0,
		gl::RGB,
		gl::UNSIGNED_BYTE,
		std::ptr::null(),
	);
	gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
	gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
	gl::FramebufferTexture2D(
		gl::FRAMEBUFFER,
		gl::COLOR_ATTACHMENT0,
		gl::TEXTURE_2D,
		texture,
		0,
	);
	gl::BindTexture(gl::TEXTURE_2D, 0);
	gl::BindFramebuffer(gl::FRAMEBUFFER, 0);

	let adm = AdmWindow {
		ident: [b'W', b'N', b'D', b'W'],
		glfw,
		window,
		fbo,
	};

	Box::leak(Box::new(adm))
}

unsafe extern "C" fn adm_swap_buffers(window_ptr: *mut AdmWindow) -> c_int {
	let window = window_ptr.as_mut().unwrap();

	let graphics =
		hook::get_symbol("_ZN11teSingletonI10clGraphicsE11sm_instanceE") as *mut *mut u16;
	let graphics = graphics.read();

	// Upscaling + black bars only if the game isnt using a saved frame
	let should_blit = if GAME_VERSION.major == GameMajor::WM3 {
		graphics.byte_offset(0x48).read() != 0
	} else {
		if graphics.byte_offset(0x54).read() != 0 {
			true
		} else {
			let graphics = graphics as *mut *mut u32;
			let buffer = graphics.byte_offset(0x0C).read();
			let buffer = if buffer.is_null() {
				graphics.byte_offset(0x08).read()
			} else {
				buffer
			};

			// If frame is saved dont blit
			buffer.byte_offset(0x04).read() == 0
		}
	};

	if should_blit {
		let (window_width, window_height) = window.window.get_size();
		let window_ar = window_width as f32 / window_height as f32;
		let ar = CONFIG.width as f32 / CONFIG.height as f32;

		let (viewport_width, viewport_height, viewport_x, viewport_y) = if window_ar > ar {
			let viewport_width: i32 = ((window_height as f32) * ar) as i32;
			let viewport_x = ((window_width - viewport_width) as f32 / 2.0) as i32;
			(viewport_width, window_height, viewport_x, 0)
		} else {
			let viewport_height = ((window_width as f32) / ar) as i32;
			let viewport_y = ((window_height - viewport_height) as f32 / 2.0) as i32;
			(window_width, viewport_height, 0, viewport_y)
		};

		gl::BindFramebuffer(gl::READ_FRAMEBUFFER, 0);
		gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, window.fbo);
		gl::BlitFramebuffer(
			0,
			0,
			CONFIG.width as i32,
			CONFIG.height as i32,
			0,
			0,
			CONFIG.width as i32,
			CONFIG.height as i32,
			gl::COLOR_BUFFER_BIT,
			gl::NEAREST,
		);

		gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
		gl::Clear(gl::COLOR_BUFFER_BIT);

		gl::BindFramebuffer(gl::READ_FRAMEBUFFER, window.fbo);
		gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, 0);
		gl::BlitFramebuffer(
			0,
			0,
			CONFIG.width as i32,
			CONFIG.height as i32,
			viewport_x,
			viewport_y,
			viewport_x + viewport_width,
			viewport_y + viewport_height,
			gl::COLOR_BUFFER_BIT,
			gl::NEAREST,
		);

		gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
	}

	limit_fps();
	window.window.swap_buffers();
	window.glfw.poll_events();
	if window.window.should_close() {
		let window = window_ptr.read().window;
		drop(window);
		platform::exit(0);
	}

	0
}

static mut NEXT_FRAME: Option<std::time::Instant> = None;

/// Waits until the next frame is due, so the game runs at `fps_limit` even
/// when vsync follows a faster monitor.
unsafe fn limit_fps() {
	use std::time::{Duration, Instant};
	if CONFIG.fps_limit == 0 {
		return;
	}
	let frame = Duration::from_secs_f64(1.0 / CONFIG.fps_limit as f64);
	let now = Instant::now();
	let next = NEXT_FRAME.unwrap_or(now);
	if next > now {
		// Sleep most of the wait, then spin for precision.
		let wait = next - now;
		if wait > Duration::from_millis(2) {
			std::thread::sleep(wait - Duration::from_millis(2));
		}
		while Instant::now() < next {
			std::hint::spin_loop();
		}
	}
	// If a frame ran long, restart the schedule instead of rushing to catch up.
	let base = if next + frame < now { now } else { next };
	NEXT_FRAME = Some(base + frame);
}

static mut CL_APP_INSTANCE: *const () = std::ptr::null();
static mut CL_APP_IS_MAIN_THREAD: *const () = std::ptr::null();
static mut CL_MAIN_INSTANCE: *const *const *const c_void = std::ptr::null();
static mut THREAD_MANAGER_CURRENT: *const () = std::ptr::null();
static mut CALL_FROM_MAIN_THREAD: *const () = std::ptr::null();

unsafe fn is_main_thread() -> bool {
	let cl_app: *const c_void = call_game(CL_APP_INSTANCE, ());
	call_game(CL_APP_IS_MAIN_THREAD, (cl_app,))
}

/// Has the game's main thread call `f(args)`.
unsafe fn call_from_main_thread(f: *const (), args: *const c_void) {
	let thread_manager = CL_MAIN_INSTANCE.read().byte_offset(0x40).read();
	let current: *const c_void = call_game(THREAD_MANAGER_CURRENT, (thread_manager,));
	call_game::<_, ()>(CALL_FROM_MAIN_THREAD, (current, f, args));
}

static mut ORIGINAL_DEL_SPRITE_MANAGER: *const () = std::ptr::null();
unsafe extern "C" fn del_sprite_manager(this: *const c_void) {
	if is_main_thread() {
		call_game::<_, ()>(ORIGINAL_DEL_SPRITE_MANAGER, (this,));
	} else {
		call_from_main_thread(del_sprite_manager as *const (), this);
	}
}

static mut ORIGINAL_SAVE_IMAGE: *const () = std::ptr::null();
unsafe extern "C" fn save_image(render_buffer: *const c_void, filepath: *const c_void) {
	if is_main_thread() {
		call_game::<_, ()>(ORIGINAL_SAVE_IMAGE, (render_buffer, filepath));
	} else {
		let args = Box::new((render_buffer, filepath));
		call_from_main_thread(save_image_main as *const (), transmute(args.as_ref()));
	}
}

unsafe extern "C" fn save_image_main(args: *const c_void) {
	let args: &(*const c_void, *const c_void) = transmute(args);
	let (render_buffer, filepath) = *args;
	if is_main_thread() {
		call_game::<_, ()>(ORIGINAL_SAVE_IMAGE, (render_buffer, filepath));
	} else {
		panic!("Not main thread!");
	}
}

static mut ORIGINAL_CREATE_TEXTURE_HANDLE: *const () = std::ptr::null();
unsafe extern "C" fn create_texture_handle(this: *const c_void, a1: i32, a2: i32) -> i32 {
	if is_main_thread() {
		call_game(ORIGINAL_CREATE_TEXTURE_HANDLE, (this, a1, a2))
	} else {
		let args = Box::new((this, a1, a2));
		call_from_main_thread(
			create_texture_handle_main as *const (),
			transmute(args.as_ref()),
		);
		1
	}
}

unsafe extern "C" fn create_texture_handle_main(args: *const c_void) {
	let args: &(*const c_void, i32, i32) = transmute(args);
	let (this, a1, a2) = *args;
	if is_main_thread() {
		call_game::<_, i32>(ORIGINAL_CREATE_TEXTURE_HANDLE, (this, a1, a2));
	} else {
		panic!("Not main thread!");
	}
}

static mut ORIGINAL_SET_TEXTURE: *const () = std::ptr::null();
unsafe extern "C" fn set_texture(this: *const c_void, a1: i32, a2: i32) -> i32 {
	if is_main_thread() {
		call_game(ORIGINAL_SET_TEXTURE, (this, a1, a2))
	} else {
		let args = Box::new((this, a1, a2));
		call_from_main_thread(set_texture_main as *const (), transmute(args.as_ref()));
		1
	}
}

unsafe extern "C" fn set_texture_main(args: *const c_void) {
	let args: &(*const c_void, i32, i32) = transmute(args);
	let (this, a1, a2) = *args;
	if is_main_thread() {
		call_game::<_, i32>(ORIGINAL_SET_TEXTURE, (this, a1, a2));
	} else {
		panic!("Not main thread!");
	}
}

static mut ORIGINAL_SET_TEXTURE_REGION: *const () = std::ptr::null();
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn set_texture_region(
	this: *const c_void,
	a1: i32,
	a2: i32,
	a3: i32,
	a4: i32,
	a5: i32,
	a6: i32,
	a7: *const c_void,
) -> i32 {
	if is_main_thread() {
		call_game(
			ORIGINAL_SET_TEXTURE_REGION,
			(this, a1, a2, a3, a4, a5, a6, a7),
		)
	} else {
		let args = Box::new((this, a1, a2, a3, a4, a5, a6, a7));
		call_from_main_thread(
			set_texture_region_main as *const (),
			transmute(args.as_ref()),
		);
		1
	}
}

unsafe extern "C" fn set_texture_region_main(args: *const c_void) {
	let args: &(*const c_void, i32, i32, i32, i32, i32, i32, *const c_void) = transmute(args);
	let (this, a1, a2, a3, a4, a5, a6, a7) = *args;
	if is_main_thread() {
		call_game::<_, i32>(
			ORIGINAL_SET_TEXTURE_REGION,
			(this, a1, a2, a3, a4, a5, a6, a7),
		);
	} else {
		panic!("Not main thread!");
	}
}

pub unsafe fn init() {
	hook::hook_symbol("admvt_setup", adachi as *const ());
	hook::hook_symbol("admShutdown", adachi as *const ());
	hook::hook_symbol("admGetString", adm_version as *const ());
	hook::hook_symbol("admGetNumDevices", adachi as *const ());
	hook::hook_symbol("admInitDevicei", adachi as *const ());
	hook::hook_symbol("admChooseModeConfigi", adm_config as *const ());
	hook::hook_symbol("admModeConfigi", adachi as *const ());
	hook::hook_symbol("admChooseFBConfigi", adm_fb_config as *const ());
	hook::hook_symbol("admCreateScreeni", adachi as *const ());
	hook::hook_symbol("admCreateGraphicsContext", adachi as *const ());
	hook::hook_symbol("admCreateWindowi", adm_window as *const ());
	hook::hook_symbol("admDisplayScreen", adachi as *const ());
	hook::hook_symbol("admMakeContextCurrent", adachi as *const ());
	hook::hook_symbol("admSwapInterval", adachi as *const ());
	hook::hook_symbol("admCursorAttribi", adachi as *const ());
	hook::hook_symbol("admGetDeviceAttribi", adachi as *const ());
	hook::hook_symbol("admSwapBuffers", adm_swap_buffers as *const ());
	hook::hook_symbol("admSetMonitorGamma", adachi as *const ());

	CL_APP_INSTANCE = hook::get_symbol("_ZN11clAppSystem11getInstanceEv");
	CL_APP_IS_MAIN_THREAD = hook::get_symbol("_ZN11clAppSystem12isMainThreadEv");
	CL_MAIN_INSTANCE =
		hook::get_symbol("_ZN11teSingletonI10teSequenceI6clMainEE11sm_instanceE") as *const _;
	THREAD_MANAGER_CURRENT = hook::get_symbol("_ZN17clNPThreadManager7currentEv");
	CALL_FROM_MAIN_THREAD =
		hook::get_symbol("_ZN10clNPThread26callFunctionFromMainThreadEPFvPvES0_");

	ORIGINAL_DEL_SPRITE_MANAGER =
		hook::hook_symbol("_ZN15clSpriteManagerD1Ev", del_sprite_manager as *const ());
	ORIGINAL_SAVE_IMAGE =
		hook::hook_symbol("_ZN14clRenderBuffer9saveImageEPKc", save_image as *const ());
	ORIGINAL_CREATE_TEXTURE_HANDLE = hook::hook_symbol(
		"_ZN24clAlchemyTextureAccessor19createTextureHandleEii",
		create_texture_handle as *const (),
	);
	ORIGINAL_SET_TEXTURE = hook::hook_symbol(
		"_ZN3Gap3Gfx19igAGLEVisualContext10setTextureEii",
		set_texture as *const (),
	);
	ORIGINAL_SET_TEXTURE_REGION = hook::hook_symbol(
		"_ZN3Gap3Gfx19igAGLEVisualContext16setTextureRegionEiiiiiiPNS0_7igImageE",
		set_texture_region as *const (),
	);
}
