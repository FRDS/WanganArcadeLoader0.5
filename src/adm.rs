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

/// Receives the driver's own account of what the game is doing wrong. The
/// engine renders the world through fixed-function and multitexture state
/// rather than Cg, so when it comes out black there is nothing in any game log
/// to explain it — only the driver knows.
extern "system" fn gl_debug_message(
	_source: u32,
	kind: u32,
	id: u32,
	severity: u32,
	_length: i32,
	message: *const c_char,
	_user: *mut c_void,
) {
	// Chatty driver hints would bury the real errors.
	if severity == gl::DEBUG_SEVERITY_NOTIFICATION {
		return;
	}
	let text = unsafe {
		if message.is_null() {
			return;
		}
		CStr::from_ptr(message).to_string_lossy().into_owned()
	};
	let kind = match kind {
		gl::DEBUG_TYPE_ERROR => "error",
		gl::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "deprecated",
		gl::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "undefined",
		gl::DEBUG_TYPE_PORTABILITY => "portability",
		gl::DEBUG_TYPE_PERFORMANCE => "performance",
		_ => "other",
	};
	let line = format!("GL {kind} [{id}]: {}", text.trim_end());
	if gl_first_time(&line) {
		platform::log(&format!("{line}  (repeats suppressed)"));
		if text.contains("ProgramStringARB") {
			log_arb_program_error();
		}
	}
}

/// The driver's parse error for the ARB program it just rejected. Only ever
/// called once, from inside the debug callback: with DEBUG_OUTPUT_SYNCHRONOUS
/// the callback runs on the calling thread while this state is still current,
/// but querying GL from inside it is re-entrant, so it is deliberately a
/// one-shot.
fn log_arb_program_error() {
	const PROGRAM_ERROR_POSITION_ARB: u32 = 0x864B;
	const PROGRAM_ERROR_STRING_ARB: u32 = 0x8874;
	unsafe {
		let mut position: i32 = -1;
		gl::GetIntegerv(PROGRAM_ERROR_POSITION_ARB, &mut position);
		let message = gl::GetString(PROGRAM_ERROR_STRING_ARB);
		let message = if message.is_null() {
			"(none)".to_string()
		} else {
			CStr::from_ptr(message as *const c_char)
				.to_string_lossy()
				.into_owned()
		};
		platform::log(&format!(
			"GL ARB program rejected at offset {position}: {}",
			message.trim_end()
		));
	}
}

const GL_MAX_DISTINCT: usize = 64;
static GL_SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
	std::sync::Mutex::new(None);

/// True the first time this exact message appears. The driver repeats itself
/// for every offending draw call, which turned one run's log into 70 MB.
fn gl_first_time(message: &str) -> bool {
	let Ok(mut seen) = GL_SEEN.lock() else {
		return false;
	};
	let seen = seen.get_or_insert_with(std::collections::HashSet::new);
	if seen.len() >= GL_MAX_DISTINCT {
		return false;
	}
	seen.insert(message.to_string())
}

/// Logs which OpenGL implementation the game ended up on. Nothing recorded
/// this before, so a rendering report couldn't say which driver produced it.
unsafe fn log_gl_info() {
	let string = |name: u32| -> String {
		let ptr = gl::GetString(name);
		if ptr.is_null() {
			"?".to_string()
		} else {
			CStr::from_ptr(ptr as *const c_char)
				.to_string_lossy()
				.into_owned()
		}
	};
	// One line each, so every line carries the log prefix and greps cleanly.
	platform::log(&format!("GL vendor: {}", string(gl::VENDOR)));
	platform::log(&format!("GL renderer: {}", string(gl::RENDERER)));
	platform::log(&format!("GL version: {}", string(gl::VERSION)));
	platform::log(&format!(
		"GLSL version: {}",
		string(gl::SHADING_LANGUAGE_VERSION)
	));
	// This engine is from 2008; a modern driver's extension string is far
	// longer than anything it was tested against, so the length is worth
	// knowing if capability detection ever looks wrong.
	let extensions = string(gl::EXTENSIONS);
	platform::log(&format!("GL extensions: {} chars", extensions.len()));
	// The engine's whole shader path is ARB assembly programs. Modern drivers
	// are entitled to drop that in favour of GLSL, and Cg will still pick an
	// arb* profile, so this is worth stating outright rather than inferring
	// it from a wall of GL_INVALID_OPERATION.
	for name in [
		"GL_ARB_vertex_program",
		"GL_ARB_fragment_program",
		"GL_ARB_vertex_shader",
		"GL_ARB_fragment_shader",
		"GL_ARB_multitexture",
	] {
		let present = extensions.split_whitespace().any(|have| have == name);
		platform::log(&format!(
			"GL {name}: {}",
			if present { "yes" } else { "NO" }
		));
	}

	if CONFIG.gl_debug {
		gl::Enable(gl::DEBUG_OUTPUT);
		gl::Enable(gl::DEBUG_OUTPUT_SYNCHRONOUS);
		gl::DebugMessageCallback(Some(gl_debug_message), std::ptr::null());
		// Everything except the notification flood, which the callback drops.
		gl::DebugMessageControl(
			gl::DONT_CARE,
			gl::DONT_CARE,
			gl::DONT_CARE,
			0,
			std::ptr::null(),
			gl::TRUE,
		);
		platform::log("GL debug output enabled");
	}
}

unsafe extern "C" fn adm_window() -> *mut AdmWindow {
	let mut glfw = glfw::init(glfw::fail_on_errors).unwrap();
	glfw.window_hint(WindowHint::Resizable(false)); // Force floating on tiling window managers
	if CONFIG.gl_debug {
		// Must be requested before the context exists.
		glfw.window_hint(WindowHint::OpenGlDebugContext(true));
	}
	let (mut window, _) = glfw.with_primary_monitor(|glfw, m| {
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
				// Asking for the monitor's current mode means GLFW picks the
				// mode already in use and never changes it, which is
				// borderless fullscreen with the frame scaled to fit. Asking
				// for the render size instead makes GLFW set the mode, so the
				// drawable matches CONFIG exactly -- no letterbox, no upscale,
				// and the capture in adm_swap_buffers cannot fall short. GLFW
				// silently substitutes the closest mode when the monitor has
				// no such mode, which is reported after creation.
				let (width, height) = if CONFIG.fullscreen_exclusive {
					(CONFIG.width, CONFIG.height)
				} else {
					(mode.width, mode.height)
				};
				glfw.create_window(
					width,
					height,
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
	// The hint above asked for a non-resizable window to keep tiling window
	// managers from tiling it, and this hands resizing back. Keep both, but
	// stop the window being dragged smaller than the render size: the game
	// rasterises into a CONFIG-sized viewport, and a shorter drawable clips
	// it before anything can be captured.
	window.set_size_limits(Some(CONFIG.width), Some(CONFIG.height), None, None);

	// What we asked for is not what we necessarily got. A fullscreen request
	// is matched against the modes the monitor actually has, and a windowed
	// one is bounded by the desktop -- a client area cannot be as tall as the
	// display once there is a title bar. Report the difference rather than
	// letting it turn into clipped rows nobody can account for.
	let (drawable_width, drawable_height) = window.get_framebuffer_size();
	platform::log(&format!(
		"adm: render {}x{}, drawable {drawable_width}x{drawable_height}, {}",
		CONFIG.width,
		CONFIG.height,
		match (CONFIG.fullscreen, CONFIG.fullscreen_exclusive) {
			(true, true) => "exclusive fullscreen",
			(true, false) => "borderless fullscreen",
			(false, _) => "windowed",
		}
	));
	if drawable_width < CONFIG.width as i32 || drawable_height < CONFIG.height as i32 {
		platform::log(
			"adm: the drawable is smaller than the render size, so part of every frame is \
			 clipped away before it can be shown -- use fullscreen, or a render size that fits",
		);
		// Only worth listing when the request could not be met, and only the
		// distinct sizes: a monitor reports the same size once per refresh
		// rate, which would otherwise be pages of near-duplicates.
		glfw.with_primary_monitor(|_, m| {
			if let Some(m) = m {
				let mut sizes: Vec<(u32, u32)> = m
					.get_video_modes()
					.iter()
					.map(|v| (v.width, v.height))
					.collect();
				sizes.sort_unstable();
				sizes.dedup();
				let list: Vec<String> = sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
				platform::log(&format!("adm: this monitor offers {}", list.join(", ")));
			}
		});
	}
	// The engine advances one simulation step per frame, so the swap interval
	// decides how fast the game plays, not just how smooth it looks. Vsync is
	// right on any display that holds 60; turn it off on one that cannot, so
	// fps_limit paces the game instead of the display dragging it under.
	glfw.set_swap_interval(if CONFIG.vsync {
		SwapInterval::Sync(1)
	} else {
		SwapInterval::None
	});
	// Both of these are invisible from outside, and between them they decide
	// the frame rate, so say which is in force rather than leave it to be
	// guessed at from how the game feels.
	platform::log(&format!(
		"adm: vsync {}, fps_limit {}",
		if CONFIG.vsync { "on" } else { "off" },
		match CONFIG.fps_limit {
			0 => "off".to_string(),
			n => n.to_string(),
		}
	));

	opengl::load_gl_funcs(&glfw);
	gl::load_with(|s| glfw.get_proc_address_raw(s));
	log_gl_info();

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
	// TexImage2D with a null pointer leaves the contents undefined, and nothing
	// else ever clears this texture -- the capture below only writes the part
	// of it the drawable could supply. Any row we cannot fill would otherwise
	// show whatever happened to be in memory for the life of the process,
	// which is how a white band from the boot screen ends up frozen across the
	// top of the game. Black makes those rows indistinguishable from letterbox.
	gl::ClearColor(0.0, 0.0, 0.0, 1.0);
	gl::Clear(gl::COLOR_BUFFER_BIT);
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
		// Pixels, not screen coordinates: the blits below are in pixels, and
		// the two differ on a display with scaling.
		let (window_width, window_height) = window.window.get_framebuffer_size();
		let window_ar = window_width as f32 / window_height as f32;

		// The game renders into the default framebuffer, so the most we can
		// capture is what the drawable actually holds. Reading beyond it is
		// undefined, and in practice leaves those rows of the FBO untouched
		// for good -- the frozen band. A window shorter than CONFIG.height
		// also means the game's own rasterisation was clipped, so there is
		// nothing there to recover.
		let captured_width = (CONFIG.width as i32).min(window_width);
		let captured_height = (CONFIG.height as i32).min(window_height);

		// Everything below works in terms of what was actually captured, not
		// what was asked for. Normally they are the same and this changes
		// nothing. When they are not, presenting the full CONFIG rectangle
		// would pad the missing rows with black and show a bar; presenting
		// the captured rectangle at its own aspect instead shows the part of
		// the frame that exists, undistorted and filling the window. The
		// content is cropped either way -- those rows were never rasterised
		// -- so the bar bought nothing. The warning below is what says so.
		let ar = captured_width as f32 / captured_height as f32;

		// Said once, not per frame. A short drawable is not recoverable --
		// the engine rasterised into a CONFIG-sized viewport that the window
		// clipped -- so the only useful response is to explain it.
		if (captured_width, captured_height) != (CONFIG.width as i32, CONFIG.height as i32)
			&& !SIZE_WARNED
		{
			SIZE_WARNED = true;
			platform::log(&format!(
				"adm: the window is {window_width}x{window_height} but the render size is {}x{}, \
				 so {} column(s) and {} row(s) of each frame are clipped away before they can be \
				 shown. A window cannot be taller than the display once it has a title bar -- use \
				 fullscreen, or a render size that fits.",
				CONFIG.width,
				CONFIG.height,
				(CONFIG.width as i32 - captured_width).max(0),
				(CONFIG.height as i32 - captured_height).max(0),
			));
		}

		// Clearing once at creation only covers rows we never reach at all.
		// Shrink the window mid-run and the rows we stop reaching are already
		// holding a valid frame, which then freezes across the top for the
		// rest of the session. Clear when the captured region changes, so
		// they go black instead -- once per change, not per frame. The game
		// owns the clear colour, so borrow it and put it back.
		if (captured_width, captured_height) != LAST_CAPTURED {
			LAST_CAPTURED = (captured_width, captured_height);
			let mut previous = [0.0f32; 4];
			gl::GetFloatv(gl::COLOR_CLEAR_VALUE, previous.as_mut_ptr());
			gl::BindFramebuffer(gl::FRAMEBUFFER, window.fbo);
			gl::ClearColor(0.0, 0.0, 0.0, 1.0);
			gl::Clear(gl::COLOR_BUFFER_BIT);
			gl::ClearColor(previous[0], previous[1], previous[2], previous[3]);
		}

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
			captured_width,
			captured_height,
			0,
			0,
			captured_width,
			captured_height,
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
			captured_width,
			captured_height,
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
	report_fps();
	window.window.swap_buffers();
	window.glfw.poll_events();
	if window.window.should_close() {
		let window = window_ptr.read().window;
		drop(window);
		platform::exit(0);
	}

	0
}

static mut FRAME_COUNT: u32 = 0;
static mut RATE_SINCE: Option<std::time::Instant> = None;

/// Reports the frame rate actually achieved, once every five seconds.
///
/// Because the engine takes one simulation step per frame, this is the game's
/// speed and not merely its smoothness, which makes it worth stating rather
/// than estimating by eye. The two things that cap it -- vsync following the
/// display, and `fps_limit` -- are both invisible from outside.
///
/// Five seconds keeps this to a dozen lines a minute, well inside the rule
/// that nothing on the per-frame path may log per frame.
unsafe fn report_fps() {
	use std::time::{Duration, Instant};
	const EVERY: Duration = Duration::from_secs(5);

	let now = Instant::now();
	let since = *RATE_SINCE.get_or_insert(now);
	FRAME_COUNT += 1;
	let elapsed = now - since;
	if elapsed < EVERY {
		return;
	}
	let fps = FRAME_COUNT as f64 / elapsed.as_secs_f64();
	platform::log(&format!(
		"adm: {fps:.1} fps over the last {:.0}s",
		elapsed.as_secs_f64()
	));
	FRAME_COUNT = 0;
	RATE_SINCE = Some(now);
}

static mut SIZE_WARNED: bool = false;
static mut LAST_CAPTURED: (i32, i32) = (0, 0);

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
