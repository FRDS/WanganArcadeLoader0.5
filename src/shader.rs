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

/// Narrows one driver answer to the extensions a caller asked about.
///
/// `probed` is which of `wanted` the driver turned out to lack, so anything
/// outside `wanted` was never put to it at all. Filtering alone would report
/// such an extension as supported -- it cannot be in `probed` -- which is a
/// conclusive yes to a question nobody asked, and on this module's one
/// decision that means a black world. `None` instead, which every caller
/// reads as "assume unsupported".
fn answer_from(
	wanted: &[String],
	probed: Option<&[String]>,
	asked: &[String],
) -> Option<Vec<String>> {
	let absent = probed?;
	if asked.iter().any(|extension| !wanted.contains(extension)) {
		return None;
	}
	Some(
		asked
			.iter()
			.filter(|extension| absent.contains(extension))
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

/// The Cg profile pair a recompile aims at.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct Profiles {
	vertex: &'static str,
	fragment: &'static str,
}

impl Profiles {
	/// ARB assembly, which every driver this game meets accepts. The fallback
	/// for every uncertain path, and the only pair allowed on a folder whose
	/// own profile we have not established.
	const PORTABLE: Self = Self {
		vertex: "arbvp1",
		fragment: "arbfp1",
	};
}

/// The profiles whose own `OPTION` lines are the ones in `declared`, or `None`
/// when that is not a set we can name profiles for -- which means portable,
/// since a profile we cannot name is one we have never tested.
///
/// Both halves have to be declared, which is what keeps the question answered
/// the same as the question asked: see `answer_from`, where an extension
/// nothing declared would come back supported without the driver ever hearing
/// about it.
///
/// Only the pair this dump ships as is listed, deliberately. cgc emits
/// vp30/fp30 as NV assembly (`!!VP2.0`, `!!FP1.0`) with no `OPTION` lines at
/// all, so `required_nv_extensions` can never produce a set that selects them;
/// and the ARB forms of that generation are gated by `_option`-suffixed
/// extension names, which `OPTION NV_x;` -> `GL_NV_x` does not produce. A row
/// nothing can reach is a row no test covers.
fn native_profiles(declared: &[String]) -> Option<Profiles> {
	let declares = |name: &str| declared.iter().any(|extension| extension == name);
	(declares("GL_NV_vertex_program3") && declares("GL_NV_fragment_program2")).then_some(Profiles {
		vertex: "vp40",
		fragment: "fp40",
	})
}

/// What one recompile pass did. Compile failures are counted apart from I/O
/// ones because only the first is worth a second pass at another profile: a
/// folder we cannot read or write fails the same way whatever we aim at.
#[derive(Default)]
struct Counts {
	written: usize,
	compile_failed: usize,
	io_failed: usize,
}

#[derive(Debug)]
enum Action {
	/// The programs already in place suit this driver.
	Leave(String),
	/// Put the `.orig` copies back.
	Restore(String),
	/// Compile the `.cg` sources, to these profiles.
	Recompile(String, Profiles),
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
		return Action::Recompile("shader_mode = portable".to_string(), Profiles::PORTABLE);
	}

	// start.sh's own recompile deletes each .fp/.vp before doing the work and
	// keeps no backup, so a run of it without cgc leaves nothing behind. The
	// .cg sources survive that, so it is recoverable -- but only if we notice
	// the compiled programs are gone, which the old content test never did.
	if state.missing > 0 {
		// Which profiles to aim at is the folder's own account of what it was
		// built as: the live programs' OPTION lines, or the `.orig` copies'
		// when no live program is left to ask.
		let declared = if state.required.is_empty() {
			&state.backup_required
		} else {
			&state.required
		};
		let profiles = match native_profiles(declared) {
			// A conclusive yes is the only thing that buys the native pair.
			// "The driver lacks one" and "could not ask" both go portable.
			Some(native) if matches!(probe(declared), Some(absent) if absent.is_empty()) => native,
			_ => Profiles::PORTABLE,
		};
		return Action::Recompile(
			format!("{} compiled programs are missing", state.missing),
			profiles,
		);
	}

	if !state.required.is_empty() {
		return match probe(&state.required) {
			Some(missing) if missing.is_empty() => Action::Leave(format!(
				"this driver has {}, so the shipped programs are the better ones",
				state.required.join(", ")
			)),
			Some(missing) => Action::Recompile(
				format!(
					"this driver lacks {}, which the shipped programs need",
					missing.join(", ")
				),
				Profiles::PORTABLE,
			),
			None => Action::Recompile(
				"could not make a context to ask the driver what it supports".to_string(),
				Profiles::PORTABLE,
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
	let probe = |asked: &[String]| answer_from(&wanted, probed.as_deref(), asked);

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
		Action::Recompile(reason, profiles) if state.sources == 0 => {
			platform::log(&format!(
				"shader: would recompile to {} / {} ({reason}) but {SHADER_DIR} has no .cg \
				 sources to compile from, so nothing can be done here",
				profiles.vertex, profiles.fragment
			));
		}
		Action::Recompile(reason, profiles) => {
			platform::log(&format!(
				"shader: recompiling to {} / {} -- {reason}",
				profiles.vertex, profiles.fragment
			));
			let counts = recompile(profiles);
			// The native pair is only ever chosen on a conclusive probe, but
			// the driver saying yes is not Cg promising it can produce that
			// profile, and a folder half-converted to one that failed is the
			// black world. Portable still works, so take it -- once, from
			// here, because recompile() must not call itself.
			if profiles != Profiles::PORTABLE && counts.compile_failed > 0 {
				platform::log(&format!(
					"shader: {} programs failed at {} / {}, falling back to portable ARB",
					counts.compile_failed, profiles.vertex, profiles.fragment
				));
				recompile(Profiles::PORTABLE);
			}
		}
	}
}

/// Compiles the `.cg` sources in data/shader to `profiles`, in place.
///
/// Portable does the whole folder: a mix of surviving NV programs and new ARB
/// ones is the black world again. A native pair touches only what is missing,
/// because a native pair is only ever chosen when the programs still in place
/// are already that profile -- those are what the dump shipped, and the
/// compiler in cg.dll is established as equivalent to cgc, not to whatever
/// built them in 2008.
unsafe fn recompile(profiles: Profiles) -> Counts {
	let mut counts = Counts::default();
	let Some(cg) = load_cg() else {
		platform::log(&format!(
			"shader: {} unavailable, cannot recompile, so the world will render \
			 black on this driver. It ships beside the loader -- put it back next \
			 to the game and launch again.",
			platform::CG_LIBRARY
		));
		return counts;
	};

	let context = (cg.create_context)();
	if context.is_null() {
		platform::log("shader: cgCreateContext failed, not recompiling");
		return counts;
	}
	let vertex_name = CString::new(profiles.vertex).unwrap();
	let fragment_name = CString::new(profiles.fragment).unwrap();
	let vertex_profile = (cg.get_profile)(vertex_name.as_ptr());
	let fragment_profile = (cg.get_profile)(fragment_name.as_ptr());
	let only_missing = profiles != Profiles::PORTABLE;

	let Ok(entries) = std::fs::read_dir(SHADER_DIR) else {
		(cg.destroy_context)(context);
		return counts;
	};
	let mut sources = 0;
	let mut left_alone = 0;
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension().and_then(|e| e.to_str()) != Some("cg") {
			continue;
		}
		sources += 1;

		// Which of the two programs this source still owes us, and how to ask
		// Cg for each. The entry point names come from the game's own .cg
		// sources; start.bat used to pass the same pair to cgc.
		let mut wanted = Vec::new();
		for (profile, name, entry_point, extension) in [
			(vertex_profile, profiles.vertex, c"v_main", "vp"),
			(fragment_profile, profiles.fragment, c"p_main", "fp"),
		] {
			let output = path.with_extension(extension);
			if only_missing && output.exists() {
				left_alone += 1;
				continue;
			}
			wanted.push((profile, name, entry_point, output));
		}
		if wanted.is_empty() {
			continue;
		}

		// Skipping silently here is what made two programs vanish for good:
		// `missing` then stays non-zero on every launch, so the capability
		// check never runs and the files are never produced. Say what broke.
		let bytes = match std::fs::read(&path) {
			Ok(bytes) => bytes,
			Err(error) => {
				counts.io_failed += wanted.len();
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
			counts.io_failed += wanted.len();
			platform::log(&format!("shader: cannot hand {} to Cg", path.display()));
			continue;
		};

		for (profile, name, entry_point, output) in wanted {
			match compile(&cg, context, &source, profile, entry_point) {
				Some(text) => match write_program(&output, &text) {
					Ok(()) => counts.written += 1,
					Err(error) => {
						counts.io_failed += 1;
						platform::log(&format!(
							"shader: cannot write {}: {error}",
							output.display()
						));
					}
				},
				None => {
					counts.compile_failed += 1;
					platform::log(&format!(
						"shader: {} failed to compile at {name}: {}",
						output.display(),
						listing(&cg, context)
					));
				}
			}
		}
	}
	(cg.destroy_context)(context);

	// Nothing reads this file any more -- both launchers' own cgc blocks are
	// gone -- but it records which profiles the folder now holds, which is the
	// one thing a folder moved between two machines cannot say for itself.
	let _ = std::fs::write(
		format!("{SHADER_DIR}/.recompiled"),
		format!("{} {}\n", profiles.vertex, profiles.fragment),
	);
	platform::log(&format!(
		"shader: {sources} .cg sources -> {} programs at {} / {}, {left_alone} left alone, {} \
		 failed to compile, {} could not be read or written",
		counts.written, profiles.vertex, profiles.fragment, counts.compile_failed, counts.io_failed
	));
	counts
}

#[cfg(test)]
mod tests {
	use super::{
		answer_from, decide, native_profiles, required_nv_extensions, Action, Profiles, ShaderState,
	};
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

	const VERTEX: &str = "GL_NV_vertex_program3";
	const FRAGMENT: &str = "GL_NV_fragment_program2";

	/// What the dump ships as: both stages built for NVIDIA. This is the pair
	/// the probe reported on the one NVIDIA machine to hand.
	fn nv_only_dump() -> ShaderState {
		ShaderState {
			sources: 31,
			missing: 0,
			required: vec![VERTEX.to_string(), FRAGMENT.to_string()],
			backup_required: Vec::new(),
			backups: 0,
		}
	}

	/// The same dump with two programs gone, which is the branch that used to
	/// return before the driver was asked anything.
	fn nv_dump_missing_programs() -> ShaderState {
		ShaderState {
			missing: 4,
			..nv_only_dump()
		}
	}

	fn native() -> Profiles {
		Profiles {
			vertex: "vp40",
			fragment: "fp40",
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

	/// A driver with one stage's extension and not the other's.
	fn supports_only_fragment(wanted: &[String]) -> Option<Vec<String>> {
		Some(
			wanted
				.iter()
				.filter(|extension| extension.as_str() != FRAGMENT)
				.cloned()
				.collect(),
		)
	}

	/// A probe that could not be run at all.
	fn inconclusive(_: &[String]) -> Option<Vec<String>> {
		None
	}

	fn profiles_of(action: &Action) -> Profiles {
		match action {
			Action::Recompile(_, profiles) => *profiles,
			other => panic!("expected a recompile, got {other:?}"),
		}
	}

	#[test]
	fn leaves_the_shipped_programs_alone_when_the_driver_can_run_them() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), supports_everything);
		assert!(matches!(action, Action::Leave(_)), "{action:?}");
	}

	#[test]
	fn recompiles_when_the_driver_lacks_the_extensions() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), supports_nothing);
		match action {
			Action::Recompile(reason, profiles) => {
				assert!(reason.contains(FRAGMENT), "{reason}");
				assert_eq!(profiles, Profiles::PORTABLE);
			}
			other => panic!("expected a recompile, got {other:?}"),
		}
	}

	#[test]
	fn an_inconclusive_probe_recompiles_rather_than_risking_a_black_world() {
		let action = decide(ShaderMode::Auto, &nv_only_dump(), inconclusive);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
	}

	#[test]
	fn rebuilds_shaders_that_are_missing_entirely() {
		// What start.sh's old recompile left behind when cgc was absent: the
		// .cg sources survive, every compiled program is gone, and so is
		// anything that said what profile the folder was built for.
		let state = ShaderState {
			sources: 31,
			missing: 62,
			..Default::default()
		};
		// Portable even on a driver that answers yes to everything: nothing
		// declared is not consent to a profile we would have to guess.
		let action = decide(ShaderMode::Auto, &state, supports_everything);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
	}

	#[test]
	fn missing_programs_rebuild_to_the_profile_they_shipped_as() {
		let state = nv_dump_missing_programs();
		let action = decide(ShaderMode::Auto, &state, supports_everything);
		assert_eq!(profiles_of(&action), native());
	}

	#[test]
	fn missing_programs_rebuild_portable_unless_the_driver_says_yes() {
		let state = nv_dump_missing_programs();
		// Could not ask.
		let action = decide(ShaderMode::Auto, &state, inconclusive);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
		// Asked, and told no.
		let action = decide(ShaderMode::Auto, &state, supports_nothing);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
		// Told yes for one stage only. A mixed pair is not on offer here.
		let action = decide(ShaderMode::Auto, &state, supports_only_fragment);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
	}

	#[test]
	fn missing_programs_fall_back_to_what_the_backups_declare() {
		// No live program left to ask, but the .orig copies still say what
		// this folder was built for.
		let state = ShaderState {
			sources: 31,
			missing: 62,
			required: Vec::new(),
			backup_required: vec![VERTEX.to_string(), FRAGMENT.to_string()],
			backups: 62,
		};
		let action = decide(ShaderMode::Auto, &state, supports_everything);
		assert_eq!(profiles_of(&action), native());
		let action = decide(ShaderMode::Auto, &state, inconclusive);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
	}

	#[test]
	fn puts_the_originals_back_when_moved_to_a_driver_that_can_use_them() {
		let state = ShaderState {
			sources: 31,
			missing: 0,
			required: Vec::new(), // what is in place is portable
			backup_required: vec![FRAGMENT.to_string()],
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
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
		// ...including in the one branch that can pick a native pair.
		let state = nv_dump_missing_programs();
		let action = decide(ShaderMode::Portable, &state, supports_everything);
		assert_eq!(profiles_of(&action), Profiles::PORTABLE);
		// `original` is handled before decide() is reached; see ensure_portable.
	}

	// The profile table, and the probe answer the choice rests on.

	#[test]
	fn maps_the_declared_options_to_the_profiles_they_came_from() {
		assert_eq!(
			native_profiles(&[VERTEX.to_string(), FRAGMENT.to_string()]),
			Some(native())
		);
	}

	#[test]
	fn half_declared_or_unknown_options_have_no_native_profile() {
		// Nothing declared: the wrecked folder, and the case that must never
		// be read as consent.
		assert_eq!(native_profiles(&[]), None);
		// One stage only. Choosing a pair from this would mean asking the
		// driver about an extension nothing in the folder declared.
		assert_eq!(native_profiles(&[VERTEX.to_string()]), None);
		assert_eq!(native_profiles(&[FRAGMENT.to_string()]), None);
		// A profile we have no name for.
		assert_eq!(native_profiles(&["GL_NV_gpu_program4".to_string()]), None);
	}

	#[test]
	fn an_unasked_extension_is_never_reported_as_supported() {
		let wanted = vec![FRAGMENT.to_string()];
		// The driver had everything it was asked about -- but it was never
		// asked about the vertex extension, so there is no answer to give.
		assert_eq!(answer_from(&wanted, Some(&[]), &[VERTEX.to_string()]), None);
		assert_eq!(
			answer_from(&wanted, Some(&[]), &[FRAGMENT.to_string()]),
			Some(Vec::new())
		);
	}

	#[test]
	fn an_answer_names_only_what_the_driver_lacked() {
		let wanted = vec![VERTEX.to_string(), FRAGMENT.to_string()];
		let absent = vec![VERTEX.to_string()];
		assert_eq!(
			answer_from(&wanted, Some(absent.as_slice()), &wanted),
			Some(vec![VERTEX.to_string()])
		);
		assert_eq!(
			answer_from(&wanted, Some(absent.as_slice()), &[FRAGMENT.to_string()]),
			Some(Vec::new())
		);
		// An inconclusive probe stays inconclusive.
		assert_eq!(answer_from(&wanted, None, &wanted), None);
	}
}
