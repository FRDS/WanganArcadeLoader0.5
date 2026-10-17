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

/// `nsCarParts::LuaNickNameGetFromModelID(emCarModelID)` -> the folder name.
unsafe extern "C" fn nickname_from_model_id(model_id: c_int) -> *const c_char {
	let name: *const c_char = call_game(ORIGINAL_NICKNAME, (model_id,));
	let text = if name.is_null() {
		"(null)".to_string()
	} else {
		CStr::from_ptr(name).to_string_lossy().into_owned()
	};
	REPORTED.say(format!("car: model id {model_id} is {text}"));
	name
}

pub unsafe fn init() {
	if !CONFIG.car_log {
		return;
	}
	ORIGINAL_NICKNAME = res::watch(
		"car",
		"_ZN10nsCarParts25LuaNickNameGetFromModelIDENS_12emCarModelIDE",
		nickname_from_model_id as *const (),
		"nsCarParts::LuaNickNameGetFromModelID",
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
