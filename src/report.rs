//! Saying something once.
//!
//! The game reports nothing about itself, so the loader has to -- and most of
//! what is worth reporting sits on a path the engine walks every frame or
//! every draw. An OpenGL debug-output build without this once produced a 70 MB
//! log in a single run, which is why deduplication is a rule here rather than a
//! nicety.
//!
//! Each subsystem keeps its own `Reported`, so a budget spent by one does not
//! silence another:
//!
//! ```ignore
//! static REPORTED: Reported = Reported::new();
//! REPORTED.say(format!("res: setViewport {width}x{height}"));
//! ```
//!
//! What this is *not* for: anything the player triggers. A message that
//! answers a keypress should arrive every time the key is pressed.

use std::collections::HashSet;
use std::sync::Mutex;

/// How many distinct messages one subsystem may say before it goes quiet.
///
/// The cap is a backstop for a message that varies -- an address, a size, a
/// result code -- where deduplicating by text alone never repeats and so never
/// stops. Going quiet is the right end: the first 64 say what is happening,
/// and the 65th would not add to them.
const MAX_REPORTED: usize = 64;

/// One subsystem's record of what it has already said.
pub struct Reported {
	// `HashSet::new` is not const, so the set arrives on first use.
	said: Mutex<Option<HashSet<String>>>,
}

impl Default for Reported {
	fn default() -> Self {
		Reported::new()
	}
}

impl Reported {
	pub const fn new() -> Self {
		Reported {
			said: Mutex::new(None),
		}
	}

	/// Logs `message`, unless it has been said before or the budget is spent.
	pub fn say(&self, message: String) {
		// A poisoned lock means a panic while reporting. Losing the line is
		// better than panicking again inside a log call.
		let Ok(mut said) = self.said.lock() else {
			return;
		};
		let said = said.get_or_insert_with(HashSet::new);
		if said.len() >= MAX_REPORTED || !said.insert(message.clone()) {
			return;
		}
		crate::platform::log(&message);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn says_each_thing_once_and_then_goes_quiet() {
		let reported = Reported::new();
		assert!(reported.said.lock().unwrap().is_none(), "allocated before use");

		reported.say(String::from("first"));
		reported.say(String::from("first"));
		reported.say(String::from("second"));
		assert_eq!(reported.said.lock().unwrap().as_ref().unwrap().len(), 2);

		// A message that never repeats must still stop, or a varying one
		// defeats the whole point.
		for i in 0..1000 {
			reported.say(format!("number {i}"));
		}
		assert_eq!(
			reported.said.lock().unwrap().as_ref().unwrap().len(),
			MAX_REPORTED
		);
	}

	/// Two subsystems do not share a budget.
	#[test]
	fn one_subsystem_going_quiet_does_not_silence_another() {
		let noisy = Reported::new();
		let quiet = Reported::new();
		for i in 0..MAX_REPORTED * 2 {
			noisy.say(format!("noise {i}"));
		}
		quiet.say(String::from("still heard"));
		assert_eq!(quiet.said.lock().unwrap().as_ref().unwrap().len(), 1);
	}
}
