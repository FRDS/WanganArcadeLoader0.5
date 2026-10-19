//! Which car the game asks for, by id and by name.
//!
//! `nsCarParts` keeps three id spaces -- `emCarID`, a BNG car id and
//! `emCarModelID` -- and converts between them. The one that matters for
//! "what is this rival driving" is
//! `LuaNickNameGetFromModelID`, which turns a model id into the folder name
//! the engine loads from: `nis_Z33_Shima`, `gem_964_38RS_Shima1st` and so on.
//!
//! This exists because three rounds of static table-reading failed to say
//! where story mode gets a rival's car. The tables that *look* like rosters
//! are not consulted -- setting all 32 entries of the one at `0x12ED3E0` to a
//! single car changed nothing on screen -- so the question is better asked of
//! the running game than of the binary.
//!
//! Behind `car_log`, and deduplicated: this sits on a path the engine uses
//! whenever it loads a car, and the rule for anything per-draw or per-load is
//! `report.rs`.

use crate::platform::call_game;
use crate::*;

static mut ORIGINAL_NICKNAME: *const () = std::ptr::null();
static mut ORIGINAL_MODEL_FROM_BNG: *const () = std::ptr::null();
static mut ORIGINAL_CAR_FROM_BNG: *const () = std::ptr::null();
static mut ORIGINAL_BNG_FROM_CAR: *const () = std::ptr::null();
static REPORTED: report::Reported = report::Reported::new();

// The conversions, logged so the *stored* id can be told from the resolved
// one. The rival in story Episode 7 resolves to model id 277
// (`nis_Z33_Shima`), but searching the data for 277 finds nothing that
// drives it -- which is what you would expect if what is stored is an
// `emCarID` or a BNG car id and 277 only appears after conversion.

unsafe extern "C" fn model_from_bng(bng: c_int) -> c_int {
	let out: c_int = call_game(ORIGINAL_MODEL_FROM_BNG, (bng,));
	REPORTED.say(format!("car: BNG id {bng} -> model id {out}"));
	out
}

unsafe extern "C" fn car_from_bng(bng: c_int) -> c_int {
	let out: c_int = call_game(ORIGINAL_CAR_FROM_BNG, (bng,));
	REPORTED.say(format!("car: BNG id {bng} -> car id {out}"));
	out
}

unsafe extern "C" fn bng_from_car(car: c_int) -> c_int {
	let out: c_int = call_game(ORIGINAL_BNG_FROM_CAR, (car,));
	REPORTED.say(format!("car: car id {car} -> BNG id {out}"));
	out
}

extern "C" {
	/// The detour targets, in `shim/call.c`: each captures its own return
	/// address -- the game's call site -- and hands it to the `*_seen`
	/// function below. Rust cannot read a return address portably, and the
	/// shim is already compiled for both targets.
	fn wal_nickname_trampoline(model_id: c_int) -> *const c_char;
	fn wal_construct_body_trampoline(self_: *const c_void, param: *const c_void) -> *const c_void;
}

static mut ORIGINAL_CONSTRUCT_BODY: *const () = std::ptr::null();

/// `clCarModelData::constructBody(clCarConstructParam const&)`, with the
/// function that called it and the head of the param it was handed.
///
/// The param already holds the model id by the time `constructBody` runs, so
/// this frame is where the id becomes visible but not where it is chosen.
/// The caller is the thing worth naming, and the first few words of the param
/// are logged so the field holding the id can be identified without guessing
/// the struct layout.
#[no_mangle]
unsafe extern "C" fn wal_construct_body_seen(
	self_: *const c_void,
	param: *const c_void,
	caller: *const (),
) -> *const c_void {
	if !param.is_null() {
		let words: Vec<String> = (0..8)
			.map(|i| {
				let v = (param as *const u8).add(i * 4).cast::<u32>().read();
				format!("{v}")
			})
			.collect();
		let who = elfsym::describe(caller as u32)
			.unwrap_or_else(|| format!("0x{:08x}", caller as usize));
		REPORTED.say(format!(
			"car: constructBody from {who}, param [{}]",
			words.join(", ")
		));
	}
	call_game(ORIGINAL_CONSTRUCT_BODY, (self_, param))
}

/// `nsCarParts::LuaNickNameGetFromModelID(emCarModelID)` -> the folder name,
/// with the game function that asked for it.
///
/// **Naming the caller is the point**, and one frame is all it gives. A stack
/// scan from the detour's own frame was tried and does not work: 256 words up
/// from `__builtin_frame_address(0)` found the immediate caller and then
/// `igAGLEVisualContext::unloadTexture`, which is residue rather than a
/// frame. A real frame-pointer walk, or a window anchored on the game's `esp`
/// rather than ours, is what that would take.
#[no_mangle]
unsafe extern "C" fn wal_nickname_seen(model_id: c_int, caller: *const ()) -> *const c_char {
	let name: *const c_char = call_game(ORIGINAL_NICKNAME, (model_id,));
	let text = if name.is_null() {
		"(null)".to_string()
	} else {
		CStr::from_ptr(name).to_string_lossy().into_owned()
	};
	// Traffic eats the budget. `report.rs` goes quiet after 64 distinct
	// messages -- which is what stops a per-frame log reaching 70 MB -- and a
	// race spawns a steady stream of colour variants (`mit_CT9A9_mr-white_perl`
	// and friends) that would exhaust it before the rival ever appears. The
	// cars worth naming are the character cars, which start at 240; the
	// player's own car and all traffic sit below it.
	const CHARACTER_CAR: c_int = 240;
	if model_id < CHARACTER_CAR {
		return name;
	}
	let who = elfsym::describe(caller as u32)
		.unwrap_or_else(|| format!("0x{:08x}", caller as usize));
	REPORTED.say(format!("car: model id {model_id} is {text}, asked for by {who}"));
	name
}

pub unsafe fn init() {
	if !CONFIG.car_log {
		return;
	}
	ORIGINAL_NICKNAME = res::watch(
		"car",
		"_ZN10nsCarParts25LuaNickNameGetFromModelIDENS_12emCarModelIDE",
		wal_nickname_trampoline as *const (),
		"nsCarParts::LuaNickNameGetFromModelID",
	);
	ORIGINAL_CONSTRUCT_BODY = res::watch(
		"car",
		"_ZN14clCarModelData13constructBodyERK19clCarConstructParam",
		wal_construct_body_trampoline as *const (),
		"clCarModelData::constructBody",
	);
	ORIGINAL_MODEL_FROM_BNG = res::watch(
		"car",
		"_ZN10nsCarParts22ModelIDGetFromBNGCarIDEi",
		model_from_bng as *const (),
		"nsCarParts::ModelIDGetFromBNGCarID",
	);
	ORIGINAL_CAR_FROM_BNG = res::watch(
		"car",
		"_ZN10nsCarParts20CarIDGetFromBNGCarIDEi",
		car_from_bng as *const (),
		"nsCarParts::CarIDGetFromBNGCarID",
	);
	ORIGINAL_BNG_FROM_CAR = res::watch(
		"car",
		"_ZN10nsCarParts20BNGCarIDGetFromCarIDENS_7emCarIDE",
		bng_from_car as *const (),
		"nsCarParts::BNGCarIDGetFromCarID",
	);
}
