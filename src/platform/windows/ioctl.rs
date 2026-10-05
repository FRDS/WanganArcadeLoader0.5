//! The Windows half of `ignore_custom_ioctls`.
//!
//! On Linux this is the `ioctl` interposer in platform/linux.rs. Under LINE
//! there is no dynamic symbol interposition, so LINE's own stub is hooked by
//! address instead, the same way `system` is in mod.rs and the file stubs are
//! in files.rs.
//!
//! **This deliberately does not call the original first, where Linux does.**
//! LINE's stub (`stubs/ioctl.cpp`) translates TIOCMGET, FIONBIO, SIOCGIF* and
//! TIOCGSERIAL/TIOCSSERIAL, and hands everything else to Cygwin's `ioctl`
//! after printing "Untranslated ioctl <n>" -- which for these two ops can only
//! fail on a Windows fd. There is nothing there worth calling. What the game
//! sees is the same either way: neither platform writes to `arg`, and both
//! report success.
//!
//! Whether the game ever gets this far is a separate question. It reaches
//! these ioctls through its JAMMA I/O serial ports, and `open("/dev/ttyM1")`
//! fails under LINE -- a full-race run logged that failure and not one
//! "Untranslated ioctl" line. So this is parity with Linux rather than a fix
//! for anything observed.

use super::files::AtomicPtrCell;
use super::line;
use crate::platform::call_game;
use crate::CONFIG;
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

/// The two ops upstream's option is named for: whatever the arcade's own
/// driver answered, nothing here implements them.
const CUSTOM_OPS: [u32; 2] = [0x5463, 0x5464];

static ORIGINAL: AtomicPtrCell = AtomicPtrCell::new();

/// One line per op rather than one per call, which is the rule for anything
/// the game can reach in a loop.
static REPORTED: [AtomicBool; CUSTOM_OPS.len()] = [AtomicBool::new(false), AtomicBool::new(false)];

unsafe extern "C" fn ioctl(fd: c_int, op: u32, arg: *mut c_void) -> c_int {
	if CONFIG.ignore_custom_ioctls {
		if let Some(index) = CUSTOM_OPS.iter().position(|custom| *custom == op) {
			if !REPORTED[index].swap(true, Ordering::Relaxed) {
				crate::wal_log!("ioctl: op {op:#x} on fd {fd} answered 0 (ignore_custom_ioctls)");
			}
			return 0;
		}
	}
	let original = ORIGINAL.get();
	if original.is_null() {
		return -1;
	}
	call_game(original, (fd, op, arg))
}

pub(super) unsafe fn init() {
	let stub = line::resolve_stub("ioctl");
	if stub.is_null() {
		crate::wal_log!("ioctl: LINE has no ioctl stub, ignore_custom_ioctls has no effect");
		return;
	}
	match line::hook(stub, ioctl as *const c_void) {
		Some(original) => {
			ORIGINAL.set(original as *const ());
			crate::wal_log!("ioctl: hooked ioctl()");
		}
		None => crate::wal_log!("ioctl: failed to hook ioctl() at {stub:p}"),
	}
}
