fn main() {
	println!("cargo:rerun-if-changed=shim/align.c");

	let mut build = cc::Build::new();
	build.file("shim/align.c").opt_level(2);
	if std::env::var_os("CARGO_FEATURE_MISALIGN_TEST").is_some() {
		build.define("WAL_MISALIGN", None);
	}
	build.compile("wal_align");
}
