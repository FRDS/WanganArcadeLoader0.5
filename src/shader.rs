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

use crate::platform;
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

/// True if any compiled shader still carries an NVIDIA-only option, which is
/// what makes the whole set unusable on other hardware.
pub fn is_nvidia_only(text: &str) -> bool {
	text.contains("OPTION NV_")
}

fn shaders_need_recompiling() -> bool {
	let Ok(entries) = std::fs::read_dir(SHADER_DIR) else {
		return false;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		let extension = path.extension().and_then(|e| e.to_str());
		if extension != Some("vp") && extension != Some("fp") {
			continue;
		}
		if std::fs::read_to_string(&path).is_ok_and(|text| is_nvidia_only(&text)) {
			return true;
		}
	}
	false
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
	CStr::from_ptr(text).to_string_lossy().trim_end().to_string()
}

/// Compiles every `.cg` in data/shader to arbvp1/arbfp1, in place.
pub unsafe fn ensure_portable() {
	if !shaders_need_recompiling() {
		return;
	}
	platform::log(
		"shader: data/shader holds NVIDIA-only programs, which every other GPU \
		 rejects. Recompiling to portable ARB with the Cg runtime.",
	);

	let Some(cg) = load_cg() else {
		platform::log(&format!(
			"shader: {} unavailable, cannot recompile. On a non-NVIDIA GPU the \
			 world will render black; install the NVIDIA Cg Toolkit so cgc is on \
			 PATH, delete {SHADER_DIR}/.recompiled and run start.bat again.",
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
	let mut written = 0;
	let mut failed = 0;
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension().and_then(|e| e.to_str()) != Some("cg") {
			continue;
		}
		let Ok(source) = std::fs::read_to_string(&path) else {
			continue;
		};
		let Ok(source) = CString::new(source) else {
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
						platform::log(&format!("shader: cannot write {}: {error}", output.display()));
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

	// So start.sh/start.bat don't run cgc over the same files next launch.
	let _ = std::fs::write(format!("{SHADER_DIR}/.recompiled"), "");
	platform::log(&format!(
		"shader: recompiled {written} programs to portable ARB, {failed} failed"
	));
}

#[cfg(test)]
mod tests {
	use super::is_nvidia_only;

	#[test]
	fn spots_the_nvidia_only_options() {
		assert!(is_nvidia_only("!!ARBvp1.0\nOPTION NV_vertex_program3;\nEND\n"));
		assert!(is_nvidia_only("!!ARBfp1.0\nOPTION NV_fragment_program2;\nEND\n"));
	}

	#[test]
	fn accepts_portable_programs() {
		assert!(!is_nvidia_only("!!ARBfp1.0\nPARAM c[9];\nEND\n"));
		// The word appearing in a comment is not an OPTION directive.
		assert!(!is_nvidia_only("!!ARBvp1.0\n# vendor NVIDIA Corporation\nEND\n"));
	}
}
