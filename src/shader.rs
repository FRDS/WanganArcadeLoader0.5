//! Recompiles the game's shaders to a profile every GPU can run.
//!
//! The dump ships `data/shader/*.fp` and `*.vp` precompiled for NVIDIA —
//! `vp40`/`fp40`, carrying `OPTION NV_vertex_program3` and basic-block labels.
//! Any other driver rejects all of them (`GL_INVALID_OPERATION` in
//! `ProgramStringARB`, "syntax error near 'BB1'"), which renders the world
//! black while the HUD, drawn without shader programs, still looks perfect.
//! The game reports nothing.
//!
//! `start.sh` and `start.bat` fix this by running cgc, but only if the NVIDIA
//! Cg Toolkit happens to be installed. It usually isn't, and the warning goes
//! to a console nobody is watching.
//!
//! No extra tooling is needed, though: `cg.dll` / `libCg.so` is already a hard
//! requirement for running the game at all, and it contains the whole Cg
//! compiler, not just the runtime. `cgCreateProgram` + `cgGetProgramString`
//! produce exactly what cgc writes. This runs before the engine starts, so the
//! files are already portable by the time anything reads them.

use crate::{platform, ShaderMode};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};

type CgContext = *mut c_void;
type CgProgram = *mut c_void;

// From Cg's cg.h.
const CG_SOURCE: c_int = 4112;
const CG_COMPILED_PROGRAM: c_int = 4106;

const SHADER_DIR: &str = "data/shader";

struct Cg {
	create_context: unsafe extern "C" fn() -> CgContext,
	destroy_context: unsafe extern "C" fn(CgContext),
	create_program: unsafe extern "C" fn(
		CgContext,
		c_int,
		*const c_char,
		c_int,
		*const c_char,
		*const *const c_char,
	) -> CgProgram,
	destroy_program: unsafe extern "C" fn(CgProgram),
	get_program_string: unsafe extern "C" fn(CgProgram, c_int) -> *const c_char,
	get_profile: unsafe extern "C" fn(*const c_char) -> c_int,
	get_last_listing: unsafe extern "C" fn(CgContext) -> *const c_char,
}

unsafe fn symbol(module: *mut c_void, name: &str) -> Option<*mut c_void> {
	let address = platform::library_symbol(module, name);
	if address.is_null() {
		platform::log(&format!(
			"shader: {} does not export {name}",
			platform::CG_LIBRARY
		));
		return None;
	}
	Some(address)
}

unsafe fn load_cg() -> Option<Cg> {
	let module = platform::try_load_library(platform::CG_LIBRARY)?;
	Some(Cg {
		create_context: std::mem::transmute(symbol(module, "cgCreateContext")?),
		destroy_context: std::mem::transmute(symbol(module, "cgDestroyContext")?),
		create_program: std::mem::transmute(symbol(module, "cgCreateProgram")?),
		destroy_program: std::mem::transmute(symbol(module, "cgDestroyProgram")?),
		get_program_string: std::mem::transmute(symbol(module, "cgGetProgramString")?),
		get_profile: std::mem::transmute(symbol(module, "cgGetProfile")?),
		get_last_listing: std::mem::transmute(symbol(module, "cgGetLastListing")?),
	})
}

/// The GL extensions a compiled program needs, read from its own `OPTION`
/// directives: `OPTION NV_fragment_program2;` needs `GL_NV_fragment_program2`.
///
/// Derived rather than hardcoded, so a dump compiled for some other NV profile
/// is still judged against the right extension instead of being assumed bad.
pub fn required_nv_extensions(text: &str) -> Vec<String> {
	let mut names = Vec::new();
	for line in text.lines() {
		let Some(rest) = line.trim().strip_prefix("OPTION ") else {
			continue;
		};
		let name = rest.trim().trim_end_matches(';').trim();
		if !name.starts_with("NV_") {
			continue;
		}
		let extension = format!("GL_{name}");
		if !names.contains(&extension) {
			names.push(extension);
		}
	}
	names
}

#[derive(Default)]
struct ShaderState {
	/// `.cg` sources, which is what we can compile from.
	sources: usize,
	/// `.vp`/`.fp` that should exist next to a `.cg` and don't.
	missing: usize,
	/// Extensions the programs currently in place require.
	required: Vec<String>,
	/// Extensions the `.orig` copies from an earlier run require.
	backup_required: Vec<String>,
	/// How many `.orig` copies exist.
	backups: usize,
}

fn push_requirements(into: &mut Vec<String>, path: &Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	for extension in required_nv_extensions(&text) {
		if !into.contains(&extension) {
			into.push(extension);
		}
	}
}

fn scan() -> ShaderState {
	let mut state = ShaderState::default();
	let Ok(entries) = std::fs::read_dir(SHADER_DIR) else {
		return state;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		match path.extension().and_then(|e| e.to_str()) {
			Some("cg") => {
				state.sources += 1;
				for extension in ["vp", "fp"] {
					if !path.with_extension(extension).exists() {
						state.missing += 1;
					}
				}
			}
			Some("vp") | Some("fp") => push_requirements(&mut state.required, &path),
			// `foo.vp.orig`, so the extension is `orig`.
			Some("orig") => {
				state.backups += 1;
				push_requirements(&mut state.backup_required, &path);
			}
			_ => {}
		}
	}
	state
}

/// Which of `extensions` this driver does **not** have, asked through a
/// throwaway invisible window because `glfwExtensionSupported` needs a current
/// context and ours runs before the game has made one.
///
/// GLFW is reference counted in the `glfw` crate — `Drop for Glfw` only calls
/// `glfwTerminate` when the last handle goes — so initialising it here and
/// dropping it again leaves `adm.rs` free to initialise its own later.
///
/// `None` means the question could not be put to the driver at all. Callers
/// must read that as "assume unsupported": portable ARB runs everywhere, so an
/// inconclusive probe has to fall back to recompiling rather than risk the
/// black world this whole module exists to prevent.
fn unsupported_extensions(extensions: &[String]) -> Option<Vec<String>> {
	// For Window::make_current.
	use glfw::Context as _;

	let mut glfw = glfw::init_no_callbacks().ok()?;
	glfw.window_hint(glfw::WindowHint::Visible(false));
	let (mut window, _events) =
		glfw.create_window(1, 1, "wal shader probe", glfw::WindowMode::Windowed)?;
	window.make_current();
	Some(
		extensions
			.iter()
			.filter(|extension| !glfw.extension_supported(extension))
			.cloned()
			.collect(),
	)
}

/// Puts the shipped programs back from the `.orig` copies an earlier run made,
/// for the case where a folder recompiled on one GPU is later run on one that
/// can use the originals.
fn restore_originals() -> usize {
	let Ok(entries) = std::fs::read_dir(SHADER_DIR) else {
		return 0;
	};
	let mut restored = 0;
	for entry in entries.flatten() {
		let backup = entry.path();
		if backup.extension().and_then(|e| e.to_str()) != Some("orig") {
			continue;
		}
		// Strip the `.orig`, leaving the `.vp`/`.fp` it was taken from.
		let target = backup.with_extension("");
		if std::fs::copy(&backup, &target).is_ok() {
			restored += 1;
		}
	}
	restored
}

/// Keeps the shipped file as `<name>.orig` the first time only, matching what
/// start.sh and start.bat do, then writes the new program.
fn write_program(path: &Path, text: &str) -> std::io::Result<()> {
	let backup = PathBuf::from(format!("{}.orig", path.display()));
	if !backup.exists() && path.exists() {
		std::fs::copy(path, &backup)?;
	}
	std::fs::write(path, text)
}

unsafe fn compile(
	cg: &Cg,
	context: CgContext,
	source: &CStr,
	profile: c_int,
	entry: &CStr,
) -> Option<String> {
	let program = (cg.create_program)(
		context,
		CG_SOURCE,
		source.as_ptr(),
		profile,
		entry.as_ptr(),
		std::ptr::null(),
	);
	if program.is_null() {
		return None;
	}
	let text = (cg.get_program_string)(program, CG_COMPILED_PROGRAM);
	let text = (!text.is_null()).then(|| CStr::from_ptr(text).to_string_lossy().into_owned());
	(cg.destroy_program)(program);
	text
}

unsafe fn listing(cg: &Cg, context: CgContext) -> String {
	let text = (cg.get_last_listing)(context);
	if text.is_null() {
		return "no listing".to_string();
	}
	CStr::from_ptr(text)
		.to_string_lossy()
		.trim_end()
		.to_string()
}

#[derive(Debug)]
enum Action {
	/// The programs already in place suit this driver.
	Leave(String),
	/// Put the `.orig` copies back.
	Restore(String),
	/// Compile the `.cg` sources to portable ARB.
	Recompile(String),
}

/// Works out what `data/shader` needs on this machine. The reason travels with
/// the action because it is the one thing a user will ask about, and the black
/// world went undiagnosed for several sessions precisely because nothing said.
///
/// `probe` answers "which of these extensions is this driver missing", and is
/// a parameter so the whole decision table can be tested without a GPU — a
/// mistake in here is a black world, which is the bug this module exists for.
fn decide(
	mode: ShaderMode,
	state: &ShaderState,
	probe: impl Fn(&[String]) -> Option<Vec<String>>,
) -> Action {
	if mode == ShaderMode::Portable {
		return Action::Recompile("shader_mode = portable".to_string());
	}

	// start.sh's own recompile deletes each .fp/.vp before doing the work and
	// keeps no backup, so a run of it without cgc leaves nothing behind. The
	// .cg sources survive that, so it is recoverable -- but only if we notice
	// the compiled programs are gone, which the old content test never did.
	if state.missing > 0 {
		return Action::Recompile(format!("{} compiled programs are missing", state.missing));
	}

	if !state.required.is_empty() {
		return match probe(&state.required) {
			Some(missing) if missing.is_empty() => Action::Leave(format!(
				"this driver has {}, so the shipped programs are the better ones",
				state.required.join(", ")
			)),
			Some(missing) => Action::Recompile(format!(
				"this driver lacks {}, which the shipped programs need",
				missing.join(", ")
			)),
			None => Action::Recompile(
				"could not make a context to ask the driver what it supports".to_string(),
			),
		};
	}

	// What is in place is already portable. If an earlier run set the shipped
	// programs aside and this driver can run them, hand them back -- they are
	// the better ones, and this is the folder-moved-between-machines case.
	//
	// This was advisory for a while, out of a worry that on a hybrid laptop
	// the probe's own window might land on a different GPU than the one the
	// game renders with. It does not: the OpenGL ICD is loaded once per
	// process, so the game inherits whatever card the probe's context brought
	// in. Confirmed on the Intel UHD + RTX 2070 Max-Q laptop in both
	// configurations -- with no GPU preference set, probe and game both
	// reported Intel; with the per-application preference set to the discrete
	// card, the same run logged "driver probe: has all of
	// GL_NV_vertex_program3, GL_NV_fragment_program2" and "GL renderer:
	// NVIDIA GeForce RTX 2070 with Max-Q Design".
	if state.backups > 0 && !state.backup_required.is_empty() {
		if let Some(absent) = probe(&state.backup_required) {
			if absent.is_empty() {
				return Action::Restore(format!(
					"this driver has {}, so the shipped programs can be used again",
					state.backup_required.join(", ")
				));
			}
		}
	}

	Action::Leave("the programs in place are already portable".to_string())
}

/// Makes `data/shader` suit this machine, before the engine reads any of it.
///
/// Three steps in order: ask the driver whether it can run the programs the
/// dump shipped, keep a copy of them if it can't, and compile portable
/// replacements from the `.cg` sources.
pub unsafe fn ensure_portable(mode: ShaderMode) {
	if mode == ShaderMode::Original {
		// Restoring is the whole point of this mode. If an earlier run put
		// the shipped programs aside, "original" means hand them back, not
		// keep the portable ones sitting there now.
		let restored = restore_originals();
		if restored > 0 {
			let _ = std::fs::write(format!("{SHADER_DIR}/.recompiled"), "original\n");
			platform::log(&format!(
				"shader: shader_mode = original, put {restored} shipped programs back from .orig"
			));
		} else {
			platform::log("shader: shader_mode = original, leaving data/shader untouched");
		}
		return;
	}

	let state = scan();
	if state.sources == 0 && state.required.is_empty() && state.backups == 0 {
		// No dump here, or nothing about it to reason over.
		return;
	}
	// Put the question to the driver once, up front, and say what came back
	// even when the decision below turns out not to need it. Without this a
	// machine that always takes the "programs are missing" branch never
	// reports whether the probe works at all, and "leaving data/shader as it
	// is" reads the same whether the driver said no or could not be asked.
	let mut wanted = state.required.clone();
	for extension in &state.backup_required {
		if !wanted.contains(extension) {
			wanted.push(extension.clone());
		}
	}
	let probed = if wanted.is_empty() {
		None
	} else {
		let answer = unsupported_extensions(&wanted);
		match &answer {
			Some(absent) if absent.is_empty() => platform::log(&format!(
				"shader: driver probe: has all of {}",
				wanted.join(", ")
			)),
			Some(absent) => platform::log(&format!(
				"shader: driver probe: lacks {} (of {})",
				absent.join(", "),
				wanted.join(", ")
			)),
			None => platform::log(
				"shader: driver probe: could not make a context to ask, assuming unsupported",
			),
		}
		answer
	};
	// decide() reuses that one answer rather than opening a second window.
	let probe = |asked: &[String]| -> Option<Vec<String>> {
		probed.as_ref().map(|absent| {
			asked
				.iter()
				.filter(|extension| absent.contains(extension))
				.cloned()
				.collect()
		})
	};

	match decide(mode, &state, probe) {
		Action::Leave(reason) => {
			platform::log(&format!("shader: leaving data/shader as it is -- {reason}"));
		}
		Action::Restore(reason) => {
			let restored = restore_originals();
			let _ = std::fs::write(format!("{SHADER_DIR}/.recompiled"), "original\n");
			platform::log(&format!(
				"shader: put {restored} shipped programs back from .orig -- {reason}"
			));
		}
		Action::Recompile(reason) if state.sources == 0 => {
			platform::log(&format!(
				"shader: would recompile ({reason}) but {SHADER_DIR} has no .cg sources to \
				 compile from, so nothing can be done here"
			));
		}
		Action::Recompile(reason) => {
			platform::log(&format!("shader: recompiling to portable ARB -- {reason}"));
			recompile();
		}
	}
}

/// Compiles every `.cg` in data/shader to arbvp1/arbfp1, in place.
unsafe fn recompile() {
	let Some(cg) = load_cg() else {
		platform::log(&format!(
			"shader: {} unavailable, cannot recompile, so the world will render \
			 black on this driver. It ships beside the loader -- put it back next \
			 to the game, delete {SHADER_DIR}/.recompiled and launch again.",
			platform::CG_LIBRARY
		));
		return;
	};

	let context = (cg.create_context)();
	if context.is_null() {
		platform::log("shader: cgCreateContext failed, not recompiling");
		return;
	}
	let vertex_profile = (cg.get_profile)(c"arbvp1".as_ptr());
	let fragment_profile = (cg.get_profile)(c"arbfp1".as_ptr());

	let Ok(entries) = std::fs::read_dir(SHADER_DIR) else {
		(cg.destroy_context)(context);
		return;
	};
	let mut sources = 0;
	let mut written = 0;
	let mut failed = 0;
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension().and_then(|e| e.to_str()) != Some("cg") {
			continue;
		}
		sources += 1;
		// Skipping silently here is what made two programs vanish for good:
		// `missing` then stays non-zero on every launch, so the capability
		// check never runs and the files are never produced. Say what broke.
		let bytes = match std::fs::read(&path) {
			Ok(bytes) => bytes,
			Err(error) => {
				failed += 2;
				platform::log(&format!("shader: cannot read {}: {error}", path.display()));
				continue;
			}
		};
		// Seven of the Export dump's nineteen sources carry EUC-JP Japanese
		// comments -- yuv.cg among them -- which are not valid UTF-8 and used to
		// lose the whole program. Every non-ASCII byte in them sits inside a
		// comment, so converting lossily only ever replaces text Cg ignores.
		if std::str::from_utf8(&bytes).is_err() {
			platform::log(&format!(
				"shader: {} is not valid UTF-8 (EUC-JP comments), converting lossily",
				path.display()
			));
		}
		let Ok(source) = CString::new(String::from_utf8_lossy(&bytes).replace('\0', "")) else {
			failed += 2;
			platform::log(&format!("shader: cannot hand {} to Cg", path.display()));
			continue;
		};

		// Entry point names come from the game's own .cg sources; start.bat
		// passes the same pair to cgc.
		for (profile, entry_point, extension) in [
			(vertex_profile, c"v_main", "vp"),
			(fragment_profile, c"p_main", "fp"),
		] {
			let output = path.with_extension(extension);
			match compile(&cg, context, &source, profile, entry_point) {
				Some(text) => match write_program(&output, &text) {
					Ok(()) => written += 1,
					Err(error) => {
						failed += 1;
						platform::log(&format!(
							"shader: cannot write {}: {error}",
							output.display()
						));
					}
				},
				None => {
					failed += 1;
					platform::log(&format!(
						"shader: {} failed to compile: {}",
						output.display(),
						listing(&cg, context)
					));
				}
			}
		}
	}
	(cg.destroy_context)(context);

	// Stops start.bat running cgc over the same files next launch. The profile
	// goes in the file so a folder moved between two different GPUs can be
	// told apart from one that has simply already been done.
	let _ = std::fs::write(format!("{SHADER_DIR}/.recompiled"), "arbvp1 arbfp1\n");
	platform::log(&format!(
		"shader: {sources} .cg sources -> recompiled {written} programs to portable ARB, {failed} failed"
	));
}

#[cfg(test)]
mod tests {
	use super::{decide, required_nv_extensions, Action, ShaderState};
	use crate::ShaderMode;

	#[test]
	fn derives_the_extension_each_option_needs() {
		// This is the real shape of the dump's programs: an ARB header plus an
		// NV OPTION, which is why the header alone tells you nothing.
		assert_eq!(
			required_nv_extensions("!!ARBvp1.0\nOPTION NV_vertex_program3;\nEND\n"),
			vec!["GL_NV_vertex_program3"]
		);
		assert_eq!(
			required_nv_extensions("!!ARBfp1.0\nOPTION NV_fragment_program2;\nEND\n"),
			vec!["GL_NV_fragment_program2"]
		);
	}

	#[test]
	fn reports_each_requirement_once_and_ignores_portable_options() {
		let text = "!!ARBfp1.0\n\
			OPTION ARB_precision_hint_fastest;\n\
			OPTION NV_fragment_program2;\n\
			OPTION NV_fragment_program2;\n\
			END\n";
		assert_eq!(
			required_nv_extensions(text),
			vec!["GL_NV_fragment_program2"]
		);
	}

	#[test]
	fn finds_nothing_to_require_in_a_portable_program() {
		assert!(required_nv_extensions("!!ARBfp1.0\nPARAM c[9];\nEND\n").is_empty());
		// The word in a comment is not an OPTION directive.
		assert!(
			required_nv_extensions("!!ARBvp1.0\n# OPTION NV_vertex_program3\nEND\n").is_empty()
		);
	}

	// The decision table. Getting any of these wrong means either a black
	// world or needlessly downgrading shaders that already worked, so every
	// branch is pinned here rather than left to a run on real hardware.

	fn nv_only_dump() -> ShaderState {
		ShaderState {
			sources: 31,
			missing: 0,
			required: vec!["GL_NV_fragment_program2".to_string()],
			backup_required: Vec::new(),
			backups: 0,
		}
	}

	/// A driver that has everything asked of it.
	fn supports_everything(_: &[String]) -> Option<Vec<String>> {
		Some(Vec::new())
	}

	/// A driver missing whatever is asked of it.
	fn supports_nothing(wanted: &[String]) -> Option<Vec<String>> {
		Some(wanted.to_vec())
	}

	/// A probe that could not be run at all.
	fn inconclusive(_: &[String]) -> Option<Vec<String>> {
		None
	}

	#[test]
	fn leaves_the_shipped_programs_alone_when_the_driver_can_run_them() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), supports_everything);
		assert!(matches!(action, Action::Leave(_)), "{action:?}");
	}

	#[test]
	fn recompiles_when_the_driver_lacks_the_extensions() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), supports_nothing);
		let Action::Recompile(reason) = action else {
			panic!("expected a recompile, got {action:?}");
		};
		assert!(reason.contains("GL_NV_fragment_program2"), "{reason}");
	}

	#[test]
	fn an_inconclusive_probe_recompiles_rather_than_risking_a_black_world() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), inconclusive);
		assert!(matches!(action, Action::Recompile(_)), "{action:?}");
	}

	#[test]
	fn rebuilds_shaders_that_are_missing_entirely() {
		// What start.sh's old recompile left behind when cgc was absent: the
		// .cg sources survive but every compiled program is gone.
		let state = ShaderState {
			sources: 31,
			missing: 62,
			..Default::default()
		};
		// Recompile even on a driver that could have run the originals.
		let action = decide(ShaderMode::Auto, &state, supports_everything);
		assert!(matches!(action, Action::Recompile(_)), "{action:?}");
	}

	#[test]
	fn puts_the_originals_back_when_moved_to_a_driver_that_can_use_them() {
		let state = ShaderState {
			sources: 31,
			missing: 0,
			required: Vec::new(), // what is in place is portable
			backup_required: vec!["GL_NV_fragment_program2".to_string()],
			backups: 62,
		};
		// The probe and the game share one OpenGL ICD per process, confirmed
		// on an Intel + NVIDIA laptop, so this can act rather than advise.
		let action = decide(ShaderMode::Auto, &state, supports_everything);
		assert!(matches!(action, Action::Restore(_)), "{action:?}");
		// ...but not when that driver still cannot run them.
		let action = decide(ShaderMode::Auto, &state, supports_nothing);
		assert!(matches!(action, Action::Leave(_)), "{action:?}");
	}

	#[test]
	fn already_portable_and_never_touched_needs_nothing() {
		let state = ShaderState {
			sources: 31,
			..Default::default()
		};
		let action = decide(ShaderMode::Auto, &state, supports_nothing);
		assert!(matches!(action, Action::Leave(_)), "{action:?}");
	}

	#[test]
	fn the_config_override_skips_the_driver_entirely() {
		// portable: recompile even though this driver handles the originals.
		let action = decide(ShaderMode::Portable, &nv_only_dump(), supports_everything);
		assert!(matches!(action, Action::Recompile(_)), "{action:?}");
		// `original` is handled before decide() is reached; see ensure_portable.
	}
}
