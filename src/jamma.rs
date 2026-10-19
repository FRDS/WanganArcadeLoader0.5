use crate::*;

pub static mut SDL_STATE: Option<poll::PollState> = None;

static mut GEAR_INDEX: u8 = 0;

const GEAR_LEFT: u32 = 1;
const GEAR_RIGHT: u32 = 2;
const GEAR_UP: u32 = 4;
const GEAR_DOWN: u32 = 8;

/// The shifter in neutral, which is where a cabinet's lever sits when nobody
/// is holding it.
const NEUTRAL: u8 = 0;

/// The emulated H-pattern position for sequential gear `index`.
///
/// **Every gear holds either UP or DOWN**, because that is how the real lever
/// works: 1/3/5 are the forward row and 2/4/6 the back one, and the position
/// switches stay closed for as long as the lever is in the gate. Only neutral
/// holds nothing. See `handle_inputs` for why that matters outside a race.
fn set_gear_bits(index: u8) -> u32 {
	match index {
		0 => 0,
		1 => GEAR_LEFT | GEAR_UP,
		2 => GEAR_LEFT | GEAR_DOWN,
		3 => GEAR_UP,
		4 => GEAR_DOWN,
		5 => GEAR_RIGHT | GEAR_UP,
		6 => GEAR_RIGHT | GEAR_DOWN,
		_ => 0,
	}
}

unsafe extern "C" fn handle_inputs(data: *mut u32) {
	if adm::GLFW_WINDOW.is_null() {
		return;
	};
	if SDL_STATE.is_none() {
		SDL_STATE = Some(
			poll::PollState::new(adm::GLFW_WINDOW, CONFIG.deadzone).unwrap(),
		);
	}
	let sdl = SDL_STATE.as_mut().unwrap();
	let keyconfig = KEYCONFIG.as_ref().unwrap();
	sdl.update();

	if sdl.is_tapped(&keyconfig.card_insert) {
		// Through `card.rs`, so this goes through the same loader the printer
		// uses -- padded or cut to one card -- and takes the same lock. It
		// used to read the file itself and reach the buffer with `get_mut()`,
		// which takes no lock at all, so whether the printer could be holding
		// it did not come up.
		card::insert();
	}

	data.byte_add(0x24).write(0);

	let mut bits = 0_u32;
	if sdl.is_down(&keyconfig.test) > 0.0 {
		bits |= 0x20000000;
	}
	if sdl.is_down(&keyconfig.service) > 0.0 {
		bits |= 0x80000000;
	}
	if sdl.is_down(&keyconfig.perspective) > 0.0 {
		bits |= 0x40;
	}
	if sdl.is_down(&keyconfig.intrude) > 0.0 {
		bits |= 0x80;
	}
	if sdl.is_down(&keyconfig.brake) > 0.0 {
		bits |= 0x20;
	}
	if sdl.is_down(&keyconfig.gas) > 0.0 {
		bits |= 0x10;
	}
	if sdl.is_tapped(&keyconfig.gear_next) {
		if GEAR_INDEX < 6 {
			GEAR_INDEX += 1;
		}
	} else if sdl.is_tapped(&keyconfig.gear_previous) {
		if GEAR_INDEX > 0 {
			GEAR_INDEX -= 1;
		}
	} else if sdl.is_tapped(&keyconfig.gear_neutral) {
		GEAR_INDEX = 0;
	} else if sdl.is_tapped(&keyconfig.gear_first) {
		GEAR_INDEX = 1;
	} else if sdl.is_tapped(&keyconfig.gear_second) {
		GEAR_INDEX = 2;
	} else if sdl.is_tapped(&keyconfig.gear_third) {
		GEAR_INDEX = 3;
	} else if sdl.is_tapped(&keyconfig.gear_fourth) {
		GEAR_INDEX = 4;
	} else if sdl.is_tapped(&keyconfig.gear_fifth) {
		GEAR_INDEX = 5;
	} else if sdl.is_tapped(&keyconfig.gear_sixth) {
		GEAR_INDEX = 6;
	}
	// The test and service buttons put the lever back in neutral.
	//
	// The test menu has no cursor switches of its own: it navigates on the
	// shifter, and `clInputDevice`'s constructor sets the auto-repeat mask at
	// `+0x1c` to 0xF -- exactly these four bits and nothing else. So
	// `clInputDevice::update()` re-fires a *held* shifter direction every 8
	// frames after a 30-frame delay, which is auto-repeat working as designed
	// on a real cabinet, where the operator has let go of the lever.
	//
	// Our lever does not spring back. `GEAR_INDEX` persists across the whole
	// run, so entering the menu still in gear fed the cursor a held direction
	// for ever -- up out of 1st, 3rd or 5th, down out of 2nd, 4th or 6th --
	// and nothing in the menu could clear it, since the gear bindings that
	// reset `GEAR_INDEX` are the arrow keys the cursor was already running
	// away from.
	if sdl.is_tapped(&keyconfig.test) || sdl.is_tapped(&keyconfig.service) {
		GEAR_INDEX = NEUTRAL;
	}
	bits |= set_gear_bits(GEAR_INDEX);
	if sdl.is_down(&keyconfig.gear_up) > 0.0 {
		GEAR_INDEX = 0;
		bits |= GEAR_UP;
	}
	if sdl.is_down(&keyconfig.gear_left) > 0.0 {
		GEAR_INDEX = 0;
		bits |= GEAR_LEFT;
	}
	if sdl.is_down(&keyconfig.gear_down) > 0.0 {
		GEAR_INDEX = 0;
		bits |= GEAR_DOWN;
	}
	if sdl.is_down(&keyconfig.gear_right) > 0.0 {
		GEAR_INDEX = 0;
		bits |= GEAR_RIGHT;
	}
	data.byte_add(0x8).write(bits);

	let n2jvio = hook::get_symbol("n2jvio") as *mut u16;
	let wheel_left = sdl.is_down(&keyconfig.wheel_left);
	let wheel_right = sdl.is_down(&keyconfig.wheel_right);
	n2jvio.byte_add(0x1A8).write(
		(i16::MAX as f32 - (wheel_left * i16::MAX as f32) + (wheel_right * i16::MAX as f32)) as u16,
	);
	data.byte_add(0x20).write(u32::from_le_bytes(
		(0.0 - wheel_left + wheel_right).to_le_bytes(),
	));

	let gas = sdl.is_down(&keyconfig.gas);
	n2jvio.byte_add(0x1AA).write((gas * i16::MAX as f32) as u16);
	data.byte_add(0x30)
		.write(u32::from_le_bytes(gas.to_le_bytes()));

	let brake = sdl.is_down(&keyconfig.brake);
	n2jvio
		.byte_add(0x1AC)
		.write((brake * i16::MAX as f32) as u16);
	data.byte_add(0x34)
		.write(u32::from_le_bytes(brake.to_le_bytes()));
}

pub unsafe fn init() {
	hook::hook_symbol("_ZN10clSystemN212initSystemN2Ev", adachi as *const ());
	hook::hook_symbol("_ZN18clInputDeviceJamma8checkUseEv", adachi as *const ());
	hook::hook_symbol(
		"_ZN18clInputDeviceJamma12handleEventsEv",
		handle_inputs as *const (),
	);
	hook::hook_symbol("_ZN16clInputDevicePad12handleEventsEv", adachi as *const ());
	hook::hook_symbol(
		"_ZN16clInputDevicePad13joyButtonDownEPN3Gap7Display12igControllerENS2_7BUTTONSE",
		adachi as *const (),
	);
	hook::hook_symbol(
		"_ZN16clInputDevicePad17joyButtonPressureEPN3Gap7Display12igControllerENS2_7BUTTONSEf",
		adachi as *const (),
	);
	hook::hook_symbol(
		"_ZN16clInputDevicePad11joyButtonUpEPN3Gap7Display12igControllerENS2_7BUTTONSE",
		adachi as *const (),
	);
	hook::hook_symbol(
		"_ZN16clInputDevicePad8joyStickEPN3Gap7Display12igControllerEtff",
		adachi as *const (),
	);

	hook::hook_symbol("n2JvioTxVsync", adachi as *const ());
	hook::hook_symbol("n2JvioAckTxVsync", adachi as *const ());
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The reason `handle_inputs` has to force neutral before the test menu
	/// sees the bits: there is no gear whose position the menu reads as "no
	/// direction", so any gear at all is a held cursor key.
	#[test]
	fn every_gear_but_neutral_holds_a_direction() {
		assert_eq!(set_gear_bits(NEUTRAL) & (GEAR_UP | GEAR_DOWN), 0);
		for gear in 1..=6 {
			let held = set_gear_bits(gear) & (GEAR_UP | GEAR_DOWN);
			assert_ne!(held, 0, "gear {gear} holds neither up nor down");
		}
		// The forward row is up, the back row is down.
		for gear in [1, 3, 5] {
			assert_eq!(set_gear_bits(gear) & GEAR_UP, GEAR_UP);
		}
		for gear in [2, 4, 6] {
			assert_eq!(set_gear_bits(gear) & GEAR_DOWN, GEAR_DOWN);
		}
	}

	/// `clInputDevice`'s constructor stores 0xF as the auto-repeat mask, so
	/// the repeating bits are the shifter's and only the shifter's. Anything
	/// else we hold -- gas, brake, view, intrude -- cannot run a cursor away.
	#[test]
	fn only_the_shifter_bits_are_the_ones_that_repeat() {
		const REPEAT_MASK: u32 = 0xF;
		assert_eq!(GEAR_LEFT | GEAR_RIGHT | GEAR_UP | GEAR_DOWN, REPEAT_MASK);
		for gear in 0..=6 {
			assert_eq!(set_gear_bits(gear) & !REPEAT_MASK, 0);
		}
	}
}
