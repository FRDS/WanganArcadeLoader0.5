use std::io::Write;

fn main() {
	println!("cargo:rerun-if-changed=shim/call.c");
	cc::Build::new()
		.file("shim/call.c")
		.opt_level(2)
		.compile("wal_call");

	if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
		println!("cargo:rerun-if-changed=src/opengl.rs");
		gen_gl_wrappers();
	}
}

/// The game calls OpenGL with the Linux (cdecl) convention, but Windows'
/// opengl32 functions are stdcall. For every function src/opengl.rs redirects,
/// generate a cdecl wrapper that calls the real stdcall function, using the
/// signatures from Khronos' gl.xml.
fn gen_gl_wrappers() {
	use gl_generator::{Api, Fallbacks, Profile, Registry};

	let opengl = std::fs::read_to_string("src/opengl.rs").unwrap();
	let start = opengl.find("const FUNCS").expect("FUNCS in src/opengl.rs");
	let end = start + opengl[start..].find("];").unwrap();
	let wanted: std::collections::BTreeSet<&str> =
		opengl[start..end].split('"').skip(1).step_by(2).collect();

	// Every extension in gl.xml, so vendor functions (NV, ATI, SGIS...) are included.
	let xml = std::str::from_utf8(khronos_api::GL_XML).unwrap();
	let extensions: Vec<&str> = xml
		.split("<extension name=\"")
		.skip(1)
		.filter_map(|rest| {
			let name = &rest[..rest.find('"').unwrap()];
			let tag = &rest[..rest.find('>').unwrap()];
			let supported = tag.split("supported=\"").nth(1)?;
			let supported = &supported[..supported.find('"').unwrap()];
			supported.split('|').any(|api| api == "gl").then_some(name)
		})
		.collect();
	let registry = Registry::new(
		Api::Gl,
		(4, 6),
		Profile::Compatibility,
		Fallbacks::All,
		extensions,
	);

	let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("gl_wrappers.rs");
	let mut dest = std::fs::File::create(out).unwrap();
	// gl.xml's types match the `gl` crate's, which is generated from the same file.
	writeln!(dest, "mod __gl_imports {{ pub use std::os::raw; }}").unwrap();
	writeln!(dest, "use gl::types;").unwrap();

	let mut arms = String::new();
	let mut found = 0;
	for cmd in &registry.cmds {
		let name = format!("gl{}", cmd.proto.ident);
		if !wanted.contains(name.as_str()) {
			continue;
		}
		found += 1;
		let join = |f: &dyn Fn(&gl_generator::Binding) -> String| {
			cmd.params.iter().map(f).collect::<Vec<_>>().join(", ")
		};
		let params = join(&|p| format!("{}: {}", p.ident, p.ty));
		let types = join(&|p| p.ty.to_string());
		let idents = join(&|p| p.ident.clone());
		let ret = if cmd.proto.ty == "()" {
			String::new()
		} else {
			format!(" -> {}", cmd.proto.ty)
		};
		writeln!(
			dest,
			"static mut REAL_{name}: usize = 0;\n\
			 #[allow(non_snake_case, clippy::all)]\n\
			 unsafe extern \"C\" fn wrap_{name}({params}){ret} {{\n\
			 \tlet real_fn: extern \"system\" fn({types}){ret} = std::mem::transmute(REAL_{name});\n\
			 \treal_fn({idents})\n\
			 }}"
		)
		.unwrap();
		arms +=
			&format!("\t\t\"{name}\" => Some((wrap_{name} as *const (), &raw mut REAL_{name})),\n");
	}
	writeln!(
		dest,
		"/// The cdecl wrapper for `name`, and the slot for the real function's address.\n\
		 pub fn wrapper(name: &str) -> Option<(*const (), *mut usize)> {{\n\
		 \tmatch name {{\n{arms}\t\t_ => None,\n\t}}\n}}"
	)
	.unwrap();
	println!(
		"cargo:warning=generated {found} of {} OpenGL cdecl wrappers",
		wanted.len()
	);
}
