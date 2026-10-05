//! The game arms a 50 Hz interval timer and never handles its signal.
//!
//! Measured, with `signal` and `sigaction` watched from before `main` ran:
//! the game calls `setitimer(ITIMER_REAL)` for every 20ms and installs a
//! handler for SIGPIPE and nothing else. No `signal(14)`, no `sigaction(14)`,
//! ever. Whatever absorbed that on the arcade's own Linux, under LINE the
//! signal eventually reaches a thread that does not block it, and SIGALRM's
//! default action takes the process with it.
//!
//! That is the "it closed by itself" ending: Windows exit code `0x0E00`, which
//! is signal 14 with no core -- no `line.exe.stackdump`, nothing from the
//! vectored handler in crash.rs, no `exit()` (all four of msys's exit
//! functions were hooked and stayed silent), and nothing in the Windows event
//! log. It reproduces on a loader build from before any of this work, so it
//! has been there all along; it was simply never visible.
//!
//! Ignoring the signal is the whole fix. Nothing in the game can be listening
//! for it, so nothing loses anything, and a version that does install its own
//! handler would simply replace ours. This runs before shutdown.rs hooks
//! `signal`, so the log does not read as though the game had done it.
//!
//! Not the other ending: a run can still die of SIGSEGV (`0x8B00`, with a
//! stackdump) inside msys-2.0.dll at `pthread_cancel`. That one is a separate
//! fault and this does nothing for it.

use super::line;
use crate::platform::call_game;
use crate::CONFIG;
use std::ffi::c_int;

/// POSIX, and the same number under Cygwin.
const SIGALRM: c_int = 14;
/// `SIG_IGN`, likewise.
const SIG_IGN: usize = 1;

pub(super) unsafe fn init() {
	if !CONFIG.ignore_alarm {
		return;
	}
	let signal = line::resolve_stub("signal");
	if signal.is_null() {
		crate::wal_log!("alarm: msys exports no signal(), SIGALRM left as it is");
		return;
	}
	let previous: usize = call_game(signal as *const (), (SIGALRM, SIG_IGN));
	crate::wal_log!(
		"alarm: SIGALRM ignored, was {previous:#x} -- the game arms a 50 Hz timer for it and \
		 installs no handler"
	);
}
