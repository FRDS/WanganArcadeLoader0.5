//! Why the process stopped.
//!
//! Nothing said, until this: closing the window reaches `platform::exit` from
//! adm.rs and the game asking to stop reaches one of msys's exit functions,
//! and neither left a line behind. A run that ended on its own was then
//! indistinguishable from one somebody closed -- and "it exited by itself,
//! maybe a crash" is exactly the question a log is supposed to answer.
//!
//! What the quiet ending turned out to be, and how the negative result here
//! settled it: the process leaves with Windows exit code `0x0E00`. Cygwin
//! encodes a normal exit as `status << 8` and a signal death as `sig << 8`
//! (with `0x80` added when it dumps core), so `0x0E00` is either `exit(14)` or
//! **signal 14**, and the two are indistinguishable from the code alone. All
//! four of msys's exit functions are hooked below and none of them ever fires,
//! which leaves only the signal -- SIGALRM, which the game arms a 50 Hz timer
//! for and never handles. alarm.rs has the fix and the measurement behind
//! it; the timer and signal watching that found it has served its purpose and
//! is in the history rather than in the way.
//!
//! A fault would not come through this module at all: an access violation goes
//! to the vectored handler in crash.rs, and a Cygwin fatal signal writes
//! `line.exe.stackdump` beside the game. A line from here means the process
//! was *asked* to stop, which is the thing those two cannot tell you.
//!
//! These hooks patch msys-2.0.dll's own functions, which made them a suspect
//! when a run faulted inside msys. They were cleared by running the same build
//! without them: the fault reproduced byte for byte, same address and same
//! registers.

use super::files::AtomicPtrCell;
use super::line;
use crate::platform::call_game;
use std::ffi::{c_int, c_void};

/// Every way out of the process that msys offers, in the order they are
/// hooked. `exit` is the one the game does *not* use.
const NAMES: [&str; 4] = ["exit", "_exit", "_Exit", "quick_exit"];

static ORIGINALS: [AtomicPtrCell; NAMES.len()] = [
	AtomicPtrCell::new(),
	AtomicPtrCell::new(),
	AtomicPtrCell::new(),
	AtomicPtrCell::new(),
];

unsafe fn leaving(index: usize, code: c_int) -> ! {
	crate::wal_log!("shutdown: {}({code}) -- the game asked to stop", NAMES[index]);
	let original = ORIGINALS[index].get();
	if !original.is_null() {
		// Never comes back: the process ends inside msys's own exit.
		call_game::<_, ()>(original, (code,));
	}
	std::process::exit(code)
}

unsafe extern "C" fn exit_0(code: c_int) -> ! {
	leaving(0, code)
}
unsafe extern "C" fn exit_1(code: c_int) -> ! {
	leaving(1, code)
}
unsafe extern "C" fn exit_2(code: c_int) -> ! {
	leaving(2, code)
}
unsafe extern "C" fn exit_3(code: c_int) -> ! {
	leaving(3, code)
}

pub(super) unsafe fn init() {
	let detours: [*const c_void; NAMES.len()] = [
		exit_0 as *const c_void,
		exit_1 as *const c_void,
		exit_2 as *const c_void,
		exit_3 as *const c_void,
	];
	// `_exit` and `_Exit` are commonly the same function, and MinHook refuses a
	// second hook on one target -- the same trap files.rs hit with open64.
	let mut hooked: Vec<*mut c_void> = Vec::new();
	let mut watching = Vec::new();

	for (index, name) in NAMES.iter().enumerate() {
		let stub = line::resolve_stub(name);
		if stub.is_null() || hooked.contains(&stub) {
			continue;
		}
		match line::hook(stub, detours[index]) {
			Some(original) => {
				ORIGINALS[index].set(original as *const ());
				hooked.push(stub);
				watching.push(*name);
			}
			None => crate::wal_log!("shutdown: failed to hook {name}() at {stub:p}"),
		}
	}
	// Said out loud, because "armed and never called" and "quietly not armed"
	// are the same silence otherwise, and the interesting result here is the
	// negative one.
	if watching.is_empty() {
		crate::wal_log!("shutdown: no exit function could be hooked, exits go unreported");
	} else {
		crate::wal_log!("shutdown: watching {}", watching.join(", "));
	}
}
