use glfw::Key as Keycode;
use phf::*;
use sdl2::controller::Button;
use sdl2::event::Event;
use sdl2::*;
use std::collections::*;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum Axis {
	LeftStickLeft,
	LeftStickUp,
	LeftStickDown,
	LeftStickRight,
	RightStickLeft,
	RightStickUp,
	RightStickDown,
	RightStickRight,
	LeftTriggerDown,
	LeftTriggerUp,
	RightTriggerDown,
	RightTriggerUp,
}

/// How a raw joystick axis becomes a 0..1 input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AxisMode {
	/// JOY_AXIS<n>+ : the positive half (steering right).
	Positive,
	/// JOY_AXIS<n>- : the negative half (steering left).
	Negative,
	/// JOY_AXIS<n>_FULL : the whole travel, resting at -1 (a pedal).
	Full,
	/// JOY_AXIS<n>_FULL_INV : the whole travel, resting at +1 (most pedals).
	FullInverted,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HatDirection {
	Up,
	Right,
	Down,
	Left,
}

impl HatDirection {
	/// SDL's hat bits.
	fn bit(self) -> u8 {
		match self {
			HatDirection::Up => 1,
			HatDirection::Right => 2,
			HatDirection::Down => 4,
			HatDirection::Left => 8,
		}
	}
}

/// Turns a raw axis value (-1..1) into a 0..1 input for `mode`, with the
/// deadzone applied.
pub fn axis_value(raw: f32, mode: AxisMode, deadzone: f32) -> f32 {
	let value = match mode {
		AxisMode::Positive => raw,
		AxisMode::Negative => -raw,
		AxisMode::Full => (raw + 1.0) / 2.0,
		AxisMode::FullInverted => (1.0 - raw) / 2.0,
	};
	if value > deadzone {
		value.min(1.0)
	} else {
		0.0
	}
}

/// A raw joystick: any wheel, pedals or stick SDL can see, mapped or not.
struct Joy {
	joystick: joystick::Joystick,
	/// Axis values at the first poll, to tell pedals from sticks in the log.
	rest: Vec<f32>,
}

#[derive(Clone, Default)]
struct JoyState {
	axes: Vec<f32>,
	buttons: Vec<bool>,
	hats: Vec<u8>,
}

#[allow(dead_code)]
pub struct PollState {
	sdl: Sdl,
	gamepad: GameControllerSubsystem,
	joystick: JoystickSubsystem,
	events: EventPump,
	/// Game controllers by instance id.
	controllers: BTreeMap<u32, controller::GameController>,
	/// Raw joysticks by slot. The slot is the <d> in JOY<d>_ bindings; a
	/// removed device leaves its slot empty so the others keep their numbers.
	joys: Vec<Option<Joy>>,
	joy_state: Vec<JoyState>,
	last_joy_state: Vec<JoyState>,
	window: *mut glfw::ffi::GLFWwindow,
	deadzone: f32,
	input_log: bool,
	keyboard_state: Vec<Keycode>,
	last_keyboard_state: Vec<Keycode>,
	button_state: Vec<Button>,
	last_button_state: Vec<Button>,
	axis_state: BTreeMap<Axis, f32>,
	last_axis_state: BTreeMap<Axis, f32>,
}

#[derive(Clone, Debug)]
pub enum KeyBinding {
	Keycode(Keycode),
	Button(Button),
	Axis(Axis),
	/// `device: None` matches every joystick.
	JoyAxis {
		device: Option<usize>,
		axis: usize,
		mode: AxisMode,
	},
	JoyButton {
		device: Option<usize>,
		button: usize,
	},
	JoyHat {
		device: Option<usize>,
		hat: usize,
		direction: HatDirection,
	},
}

pub struct KeyBindings {
	keys: Vec<KeyBinding>,
}

const MAPPINGS: Map<&str, KeyBinding> = phf_map! {
	"F1" => KeyBinding::Keycode(Keycode::F1),
	"F2" => KeyBinding::Keycode(Keycode::F2),
	"F3" => KeyBinding::Keycode(Keycode::F3),
	"F4" => KeyBinding::Keycode(Keycode::F4),
	"F5" => KeyBinding::Keycode(Keycode::F5),
	"F6" => KeyBinding::Keycode(Keycode::F6),
	"F7" => KeyBinding::Keycode(Keycode::F7),
	"F8" => KeyBinding::Keycode(Keycode::F8),
	"F9" => KeyBinding::Keycode(Keycode::F9),
	"F10" => KeyBinding::Keycode(Keycode::F10),
	"F11" => KeyBinding::Keycode(Keycode::F11),
	"F12" => KeyBinding::Keycode(Keycode::F12),
	"NUM0" => KeyBinding::Keycode(Keycode::Num0),
	"NUM1" => KeyBinding::Keycode(Keycode::Num1),
	"NUM2" => KeyBinding::Keycode(Keycode::Num2),
	"NUM3" => KeyBinding::Keycode(Keycode::Num3),
	"NUM4" => KeyBinding::Keycode(Keycode::Num4),
	"NUM5" => KeyBinding::Keycode(Keycode::Num5),
	"NUM6" => KeyBinding::Keycode(Keycode::Num6),
	"NUM7" => KeyBinding::Keycode(Keycode::Num7),
	"NUM8" => KeyBinding::Keycode(Keycode::Num8),
	"NUM9" => KeyBinding::Keycode(Keycode::Num9),
	"UPARROW" => KeyBinding::Keycode(Keycode::Up),
	"LEFTARROW" => KeyBinding::Keycode(Keycode::Left),
	"DOWNARROW" => KeyBinding::Keycode(Keycode::Down),
	"RIGHTARROW" => KeyBinding::Keycode(Keycode::Right),
	"ENTER" => KeyBinding::Keycode(Keycode::Enter),
	"SPACE" => KeyBinding::Keycode(Keycode::Space),
	"CONTROL" => KeyBinding::Keycode(Keycode::LeftControl),
	"SHIFT" => KeyBinding::Keycode(Keycode::LeftShift),
	"TAB" => KeyBinding::Keycode(Keycode::Tab),
	"ESCAPE" => KeyBinding::Keycode(Keycode::Escape),
	"A" => KeyBinding::Keycode(Keycode::A),
	"B" => KeyBinding::Keycode(Keycode::B),
	"C" => KeyBinding::Keycode(Keycode::C),
	"D" => KeyBinding::Keycode(Keycode::D),
	"E" => KeyBinding::Keycode(Keycode::E),
	"F" => KeyBinding::Keycode(Keycode::F),
	"G" => KeyBinding::Keycode(Keycode::G),
	"H" => KeyBinding::Keycode(Keycode::H),
	"I" => KeyBinding::Keycode(Keycode::I),
	"J" => KeyBinding::Keycode(Keycode::J),
	"K" => KeyBinding::Keycode(Keycode::K),
	"L" => KeyBinding::Keycode(Keycode::L),
	"M" => KeyBinding::Keycode(Keycode::M),
	"N" => KeyBinding::Keycode(Keycode::N),
	"O" => KeyBinding::Keycode(Keycode::O),
	"P" => KeyBinding::Keycode(Keycode::P),
	"Q" => KeyBinding::Keycode(Keycode::Q),
	"R" => KeyBinding::Keycode(Keycode::R),
	"S" => KeyBinding::Keycode(Keycode::S),
	"T" => KeyBinding::Keycode(Keycode::T),
	"U" => KeyBinding::Keycode(Keycode::U),
	"V" => KeyBinding::Keycode(Keycode::V),
	"W" => KeyBinding::Keycode(Keycode::W),
	"X" => KeyBinding::Keycode(Keycode::X),
	"Y" => KeyBinding::Keycode(Keycode::Y),
	"Z" => KeyBinding::Keycode(Keycode::Z),
	"SDL_A" => KeyBinding::Button(Button::A),
	"SDL_B" => KeyBinding::Button(Button::B),
	"SDL_X" => KeyBinding::Button(Button::X),
	"SDL_Y" => KeyBinding::Button(Button::Y),
	"SDL_BACK" => KeyBinding::Button(Button::Back),
	"SDL_GUIDE" => KeyBinding::Button(Button::Guide),
	"SDL_START" => KeyBinding::Button(Button::Start),
	"SDL_LSHOULDER" => KeyBinding::Button(Button::LeftShoulder),
	"SDL_RSHOULDER" => KeyBinding::Button(Button::RightShoulder),
	"SDL_DPAD_UP" => KeyBinding::Button(Button::DPadUp),
	"SDL_DPAD_LEFT" => KeyBinding::Button(Button::DPadLeft),
	"SDL_DPAD_DOWN" => KeyBinding::Button(Button::DPadDown),
	"SDL_DPAD_RIGHT" => KeyBinding::Button(Button::DPadRight),
	"SDL_MISC" => KeyBinding::Button(Button::Misc1),
	"SDL_PADDLE1" => KeyBinding::Button(Button::Paddle1),
	"SDL_PADDLE2" => KeyBinding::Button(Button::Paddle2),
	"SDL_PADDLE3" => KeyBinding::Button(Button::Paddle3),
	"SDL_PADDLE4" => KeyBinding::Button(Button::Paddle4),
	"SDL_TOUCHPAD" => KeyBinding::Button(Button::Touchpad),
	"SDL_LSTICK_PRESS" => KeyBinding::Button(Button::LeftStick),
	"SDL_RSTICK_PRESS" => KeyBinding::Button(Button::RightStick),
	"SDL_LSTICK_LEFT" => KeyBinding::Axis(Axis::LeftStickLeft),
	"SDL_LSTICK_UP" => KeyBinding::Axis(Axis::LeftStickUp),
	"SDL_LSTICK_DOWN" => KeyBinding::Axis(Axis::LeftStickDown),
	"SDL_LSTICK_RIGHT" => KeyBinding::Axis(Axis::LeftStickRight),
	"SDL_RSTICK_LEFT" => KeyBinding::Axis(Axis::RightStickLeft),
	"SDL_RSTICK_UP" => KeyBinding::Axis(Axis::RightStickUp),
	"SDL_RSTICK_DOWN" => KeyBinding::Axis(Axis::RightStickDown),
	"SDL_RSTICK_RIGHT" => KeyBinding::Axis(Axis::RightStickRight),
	"SDL_LTRIGGER_DOWN" => KeyBinding::Axis(Axis::LeftTriggerDown),
	"SDL_LTRIGGER_UP" => KeyBinding::Axis(Axis::LeftTriggerUp),
	"SDL_RTRIGGER_DOWN" => KeyBinding::Axis(Axis::RightTriggerDown),
	"SDL_RTRIGGER_UP" => KeyBinding::Axis(Axis::RightTriggerUp),
};

pub fn parse_keybinding(toml: Vec<String>) -> KeyBindings {
	let mut keybindings = KeyBindings { keys: Vec::new() };
	for value in toml {
		if let Some(keybinding) = MAPPINGS.get(&value) {
			keybindings.keys.push(keybinding.clone());
		} else if let Some(keybinding) = parse_joy_binding(&value) {
			keybindings.keys.push(keybinding);
		} else {
			panic!(
				"Incorrect keybinding {value}. Raw joystick bindings look like JOY_BUTTON3, \
				 JOY_AXIS0+, JOY_AXIS0-, JOY_AXIS2_FULL, JOY_AXIS2_FULL_INV or JOY_HAT0_UP, \
				 optionally with a device number: JOY1_AXIS2_FULL_INV"
			);
		}
	}
	keybindings
}

/// Parses JOY[<device>]_BUTTON<n>, JOY[<device>]_AXIS<n>{+,-,_FULL,_FULL_INV}
/// and JOY[<device>]_HAT<n>_{UP,RIGHT,DOWN,LEFT}.
fn parse_joy_binding(value: &str) -> Option<KeyBinding> {
	let rest = value.strip_prefix("JOY")?;
	let (device, rest) = rest.split_once('_')?;
	let device = if device.is_empty() {
		None
	} else {
		Some(device.parse().ok()?)
	};

	fn number(text: &str) -> Option<usize> {
		if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
			return None;
		}
		text.parse().ok()
	}

	if let Some(button) = rest.strip_prefix("BUTTON") {
		return Some(KeyBinding::JoyButton {
			device,
			button: number(button)?,
		});
	}
	if let Some(axis) = rest.strip_prefix("AXIS") {
		let (axis, mode) = if let Some(axis) = axis.strip_suffix("_FULL_INV") {
			(axis, AxisMode::FullInverted)
		} else if let Some(axis) = axis.strip_suffix("_FULL") {
			(axis, AxisMode::Full)
		} else if let Some(axis) = axis.strip_suffix('+') {
			(axis, AxisMode::Positive)
		} else if let Some(axis) = axis.strip_suffix('-') {
			(axis, AxisMode::Negative)
		} else {
			return None;
		};
		return Some(KeyBinding::JoyAxis {
			device,
			axis: number(axis)?,
			mode,
		});
	}
	if let Some(hat) = rest.strip_prefix("HAT") {
		let (hat, direction) = hat.split_once('_')?;
		let direction = match direction {
			"UP" => HatDirection::Up,
			"RIGHT" => HatDirection::Right,
			"DOWN" => HatDirection::Down,
			"LEFT" => HatDirection::Left,
			_ => return None,
		};
		return Some(KeyBinding::JoyHat {
			device,
			hat: number(hat)?,
			direction,
		});
	}
	None
}

/// The keyconfig name of a key, pad button or pad axis, for the input log.
fn binding_name(binding: &KeyBinding) -> String {
	for (name, entry) in MAPPINGS.entries() {
		let same = match (entry, binding) {
			(KeyBinding::Keycode(a), KeyBinding::Keycode(b)) => a == b,
			(KeyBinding::Button(a), KeyBinding::Button(b)) => a == b,
			(KeyBinding::Axis(a), KeyBinding::Axis(b)) => a == b,
			_ => false,
		};
		if same {
			return name.to_string();
		}
	}
	format!("{binding:?}")
}

impl PollState {
	/// Keyboard input is read from the game's GLFW window; SDL handles
	/// controllers (as game controllers) and every joystick (as raw axes,
	/// buttons and hats, for wheels and pedals).
	pub fn new(
		window: *mut glfw::ffi::GLFWwindow,
		axis_deadzone: f32,
		input_log: bool,
	) -> Result<Self, String> {
		// SDL has no window of its own here, so don't let it drop input when
		// it thinks the application is in the background.
		sdl2::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
		let sdl = sdl2::init()?;
		let joystick = sdl.joystick()?;
		let gamepad = sdl.game_controller()?;
		let events = sdl.event_pump()?;

		if let Err(err) = gamepad.load_mappings("gamecontrollerdb.txt") {
			crate::platform::log(&format!(
				"input: gamecontrollerdb.txt not loaded ({err}), using SDL's built-in pad mappings"
			));
		}

		let mut state = Self {
			sdl,
			gamepad,
			joystick,
			events,
			controllers: BTreeMap::new(),
			joys: Vec::new(),
			joy_state: Vec::new(),
			last_joy_state: Vec::new(),
			window,
			deadzone: axis_deadzone,
			input_log,
			keyboard_state: Vec::with_capacity(255),
			last_keyboard_state: Vec::with_capacity(255),
			button_state: Vec::with_capacity(32),
			last_button_state: Vec::with_capacity(32),
			axis_state: BTreeMap::new(),
			last_axis_state: BTreeMap::new(),
		};
		let count = state.joystick.num_joysticks()?;
		crate::platform::log(&format!("input: {count} controller(s) found"));
		for index in 0..count {
			state.add_device(index);
		}
		Ok(state)
	}

	fn log(&self, msg: &str) {
		if self.input_log {
			crate::platform::log(&format!("input: {msg}"));
		}
	}

	/// Opens device `index` as a raw joystick, and as a game controller if SDL
	/// has a mapping for it. Devices already open are skipped.
	fn add_device(&mut self, index: u32) {
		match self.joystick.open(index) {
			Ok(joystick) => {
				let id = joystick.instance_id();
				let already_open = self
					.joys
					.iter()
					.flatten()
					.any(|joy| joy.joystick.instance_id() == id);
				if !already_open {
					let rest = Vec::new();
					let slot = match self.joys.iter().position(Option::is_none) {
						Some(slot) => slot,
						None => {
							self.joys.push(None);
							self.joy_state.push(JoyState::default());
							self.last_joy_state.push(JoyState::default());
							self.joys.len() - 1
						}
					};
					crate::platform::log(&format!(
						"input: JOY{slot} = \"{}\" ({} axes, {} buttons, {} hats){}",
						joystick.name(),
						joystick.num_axes(),
						joystick.num_buttons(),
						joystick.num_hats(),
						if self.gamepad.is_game_controller(index) {
							", also usable with the SDL_ pad bindings"
						} else {
							""
						}
					));
					self.joys[slot] = Some(Joy { joystick, rest });
				}
			}
			Err(err) => crate::platform::log(&format!("input: can't open device {index}: {err}")),
		}

		if self.gamepad.is_game_controller(index) {
			match self.gamepad.open(index) {
				Ok(controller) => {
					self.controllers
						.insert(controller.instance_id(), controller);
				}
				Err(err) => crate::platform::log(&format!("input: can't open pad {index}: {err}")),
			}
		}
	}

	fn remove_device(&mut self, id: u32) {
		self.controllers.remove(&id);
		for (slot, joy) in self.joys.iter_mut().enumerate() {
			if joy
				.as_ref()
				.is_some_and(|joy| joy.joystick.instance_id() == id)
			{
				crate::platform::log(&format!("input: JOY{slot} disconnected"));
				*joy = None;
				self.joy_state[slot] = JoyState::default();
			}
		}
	}

	pub fn update(&mut self) {
		self.last_keyboard_state.clear();
		self.last_button_state.clear();
		self.last_axis_state.clear();

		self.last_keyboard_state.extend(&self.keyboard_state);
		self.last_button_state.extend(&self.button_state);
		self.last_axis_state.extend(&self.axis_state);
		self.last_joy_state.clone_from(&self.joy_state);

		// GLFW only reports keys while the game window has focus.
		self.keyboard_state.clear();
		// No window yet (or in tests): no keyboard.
		if !self.window.is_null() {
			for binding in MAPPINGS.values() {
				if let KeyBinding::Keycode(key) = binding {
					let pressed = unsafe {
						glfw::ffi::glfwGetKey(self.window, *key as i32) == glfw::ffi::PRESS
					};
					if pressed && !self.keyboard_state.contains(key) {
						self.keyboard_state.push(*key);
						if !self.last_keyboard_state.contains(key) {
							self.log(&format!("{} pressed", binding_name(binding)));
						}
					}
				}
			}
		}

		let events: Vec<Event> = self.events.poll_iter().collect();
		for event in events {
			match event {
				Event::JoyDeviceAdded { which, .. } => self.add_device(which),
				Event::JoyDeviceRemoved { which, .. } => self.remove_device(which),
				Event::ControllerButtonDown { button, .. } => {
					if !self.button_state.contains(&button) {
						self.button_state.push(button);
						self.log(&format!(
							"{} pressed",
							binding_name(&KeyBinding::Button(button))
						));
					}
				}
				Event::ControllerButtonUp { button, .. } => {
					self.button_state.retain(|b| b != &button);
				}
				Event::ControllerAxisMotion { axis, value, .. } => {
					let value = normalize(value);
					use Axis::*;
					let (axis_positive, axis_negative) = match axis {
						controller::Axis::LeftX => (LeftStickRight, LeftStickLeft),
						controller::Axis::LeftY => (LeftStickDown, LeftStickUp),
						controller::Axis::RightX => (RightStickRight, RightStickLeft),
						controller::Axis::RightY => (RightStickDown, RightStickUp),
						controller::Axis::TriggerLeft => (LeftTriggerDown, LeftTriggerUp),
						controller::Axis::TriggerRight => (RightTriggerDown, RightTriggerUp),
					};
					let (positive, negative) = if value > self.deadzone {
						(value, 0.0)
					} else if value < -self.deadzone {
						(0.0, -value)
					} else {
						(0.0, 0.0)
					};
					for (axis, value) in [(axis_positive, positive), (axis_negative, negative)] {
						let was = self.axis_state.get(&axis).copied().unwrap_or(0.0);
						if was <= 0.5 && value > 0.5 {
							self.log(&format!("{} pushed", binding_name(&KeyBinding::Axis(axis))));
						}
						self.axis_state.insert(axis, value);
					}
				}
				_ => {}
			}
		}

		self.poll_joysticks();
	}

	/// Reads every raw joystick's current axes, buttons and hats. The event
	/// pump above has already updated SDL's state.
	fn poll_joysticks(&mut self) {
		for slot in 0..self.joys.len() {
			let Some(joy) = &self.joys[slot] else {
				continue;
			};
			let joystick = &joy.joystick;
			let state = JoyState {
				axes: (0..joystick.num_axes())
					.map(|axis| normalize(joystick.axis(axis).unwrap_or(0)))
					.collect(),
				buttons: (0..joystick.num_buttons())
					.map(|button| joystick.button(button).unwrap_or(false))
					.collect(),
				hats: (0..joystick.num_hats())
					.map(|hat| joystick.hat(hat).map_or(0, |hat| hat.to_raw()))
					.collect(),
			};
			if let Some(joy) = &mut self.joys[slot] {
				if joy.rest.is_empty() {
					joy.rest.clone_from(&state.axes);
				}
			}
			if self.input_log {
				self.log_joystick_changes(slot, &state);
			}
			self.joy_state[slot] = state;
		}
	}

	/// Logs what a wheel or pedal sends, with the binding names to use.
	fn log_joystick_changes(&self, slot: usize, state: &JoyState) {
		let last = &self.joy_state[slot];
		let rest = self.joys[slot]
			.as_ref()
			.map_or(&[][..], |joy| &joy.rest[..]);
		for (axis, &value) in state.axes.iter().enumerate() {
			let bucket = |v: f32| (v > 0.5) as i8 - (v < -0.5) as i8;
			let before = last.axes.get(axis).copied().unwrap_or(0.0);
			if bucket(value) == bucket(before) {
				continue;
			}
			let rest = rest.get(axis).copied().unwrap_or(0.0);
			let hint = if rest < -0.9 {
				format!("a pedal resting at -1? use JOY{slot}_AXIS{axis}_FULL")
			} else if rest > 0.9 {
				format!("a pedal resting at +1? use JOY{slot}_AXIS{axis}_FULL_INV")
			} else {
				format!("steering/stick: JOY{slot}_AXIS{axis}+ and JOY{slot}_AXIS{axis}-")
			};
			self.log(&format!("JOY{slot}_AXIS{axis} = {value:.2} ({hint})"));
		}
		for (button, &down) in state.buttons.iter().enumerate() {
			if down && !last.buttons.get(button).copied().unwrap_or(false) {
				self.log(&format!("JOY{slot}_BUTTON{button} pressed"));
			}
		}
		for (hat, &bits) in state.hats.iter().enumerate() {
			let before = last.hats.get(hat).copied().unwrap_or(0);
			for direction in [
				HatDirection::Up,
				HatDirection::Right,
				HatDirection::Down,
				HatDirection::Left,
			] {
				if bits & direction.bit() != 0 && before & direction.bit() == 0 {
					let name = format!("{direction:?}").to_uppercase();
					self.log(&format!("JOY{slot}_HAT{hat}_{name} pressed"));
				}
			}
		}
	}

	/// A raw joystick binding's value in `states`, over every device unless
	/// the binding names one.
	fn joy_value(&self, states: &[JoyState], binding: &KeyBinding) -> f32 {
		let device = match binding {
			KeyBinding::JoyAxis { device, .. }
			| KeyBinding::JoyButton { device, .. }
			| KeyBinding::JoyHat { device, .. } => *device,
			_ => return 0.0,
		};
		let value = |state: &JoyState| -> f32 {
			match binding {
				KeyBinding::JoyAxis { axis, mode, .. } => state
					.axes
					.get(*axis)
					.map_or(0.0, |raw| axis_value(*raw, *mode, self.deadzone)),
				KeyBinding::JoyButton { button, .. } => {
					state.buttons.get(*button).copied().unwrap_or(false) as i32 as f32
				}
				KeyBinding::JoyHat { hat, direction, .. } => state
					.hats
					.get(*hat)
					.map_or(0.0, |bits| (bits & direction.bit() != 0) as i32 as f32),
				_ => 0.0,
			}
		};
		match device {
			Some(device) => states.get(device).map_or(0.0, value),
			None => states.iter().map(value).fold(0.0, f32::max),
		}
	}

	fn keycode_is_down(&self, keycode: &Keycode) -> bool {
		self.keyboard_state.contains(keycode)
	}
	fn keycode_was_down(&self, keycode: &Keycode) -> bool {
		self.last_keyboard_state.contains(keycode)
	}

	fn button_is_down(&self, button: &Button) -> bool {
		self.button_state.contains(button)
	}
	fn button_was_down(&self, button: &Button) -> bool {
		self.last_button_state.contains(button)
	}

	fn axis_is_down(&self, axis: &Axis) -> f32 {
		*self.axis_state.get(axis).unwrap_or(&0.0)
	}
	fn axis_was_down(&self, axis: &Axis) -> f32 {
		*self.last_axis_state.get(axis).unwrap_or(&0.0)
	}

	fn binding_is_down(&self, keybinding: &KeyBinding) -> f32 {
		match keybinding {
			KeyBinding::Keycode(keycode) => self.keycode_is_down(keycode) as i32 as f32,
			KeyBinding::Button(button) => self.button_is_down(button) as i32 as f32,
			KeyBinding::Axis(axis) => self.axis_is_down(axis),
			joy => self.joy_value(&self.joy_state, joy),
		}
	}
	fn binding_is_up(&self, keybinding: &KeyBinding) -> bool {
		self.binding_is_down(keybinding) == 0.0
	}
	fn binding_was_down(&self, keybinding: &KeyBinding) -> f32 {
		match keybinding {
			KeyBinding::Keycode(keycode) => self.keycode_was_down(keycode) as i32 as f32,
			KeyBinding::Button(button) => self.button_was_down(button) as i32 as f32,
			KeyBinding::Axis(axis) => self.axis_was_down(axis),
			joy => self.joy_value(&self.last_joy_state, joy),
		}
	}
	fn binding_was_up(&self, keybinding: &KeyBinding) -> bool {
		self.binding_was_down(keybinding) == 0.0
	}
	fn binding_is_tapped(&self, keybinding: &KeyBinding) -> bool {
		self.binding_is_down(keybinding) != 0.0 && self.binding_was_up(keybinding)
	}
	fn binding_is_released(&self, keybinding: &KeyBinding) -> bool {
		self.binding_was_down(keybinding) != 0.0 && self.binding_is_up(keybinding)
	}

	pub fn is_down(&self, keybindings: &KeyBindings) -> f32 {
		for keybinding in keybindings.keys.iter() {
			let value = self.binding_is_down(keybinding);
			if value > 0.0 {
				return value;
			}
		}
		0.0
	}
	pub fn is_up(&self, keybindings: &KeyBindings) -> bool {
		for keybinding in keybindings.keys.iter() {
			if self.binding_is_up(keybinding) {
				return true;
			}
		}
		false
	}
	pub fn was_down(&self, keybindings: &KeyBindings) -> f32 {
		for keybinding in keybindings.keys.iter() {
			let value = self.binding_was_down(keybinding);
			if value > 0.0 {
				return value;
			}
		}
		0.0
	}
	pub fn was_up(&self, keybindings: &KeyBindings) -> bool {
		for keybinding in keybindings.keys.iter() {
			if self.binding_was_up(keybinding) {
				return true;
			}
		}
		false
	}
	pub fn is_tapped(&self, keybindings: &KeyBindings) -> bool {
		for keybinding in keybindings.keys.iter() {
			if self.binding_is_tapped(keybinding) {
				return true;
			}
		}
		false
	}
	pub fn is_released(&self, keybindings: &KeyBindings) -> bool {
		for keybinding in keybindings.keys.iter() {
			if self.binding_is_released(keybinding) {
				return true;
			}
		}
		false
	}
}

/// SDL axis value (-32768..32767) as -1..1.
fn normalize(value: i16) -> f32 {
	(value as f32 / i16::MAX as f32).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parse(value: &str) -> KeyBinding {
		parse_joy_binding(value).unwrap_or_else(|| panic!("{value} didn't parse"))
	}

	#[test]
	fn parses_joystick_bindings() {
		assert!(matches!(
			parse("JOY_BUTTON3"),
			KeyBinding::JoyButton {
				device: None,
				button: 3
			}
		));
		assert!(matches!(
			parse("JOY1_BUTTON12"),
			KeyBinding::JoyButton {
				device: Some(1),
				button: 12
			}
		));
		assert!(matches!(
			parse("JOY_AXIS0+"),
			KeyBinding::JoyAxis {
				device: None,
				axis: 0,
				mode: AxisMode::Positive
			}
		));
		assert!(matches!(
			parse("JOY_AXIS0-"),
			KeyBinding::JoyAxis {
				device: None,
				axis: 0,
				mode: AxisMode::Negative
			}
		));
		assert!(matches!(
			parse("JOY2_AXIS5_FULL"),
			KeyBinding::JoyAxis {
				device: Some(2),
				axis: 5,
				mode: AxisMode::Full
			}
		));
		assert!(matches!(
			parse("JOY_AXIS2_FULL_INV"),
			KeyBinding::JoyAxis {
				device: None,
				axis: 2,
				mode: AxisMode::FullInverted
			}
		));
		assert!(matches!(
			parse("JOY0_HAT0_LEFT"),
			KeyBinding::JoyHat {
				device: Some(0),
				hat: 0,
				direction: HatDirection::Left
			}
		));
		for bad in [
			"JOY_AXIS0",
			"JOY_AXIS+",
			"JOY_AXISX+",
			"JOY_BUTTON",
			"JOY_BUTTON-1",
			"JOYX_BUTTON1",
			"JOY_HAT0",
			"JOY_HAT0_SIDEWAYS",
			"JOY_TRIGGER1",
			"JOYBUTTON1",
		] {
			assert!(parse_joy_binding(bad).is_none(), "{bad} should be rejected");
		}
	}

	#[test]
	fn keyconfig_names_still_parse() {
		let keys = parse_keybinding(vec![
			"W".into(),
			"SDL_A".into(),
			"JOY_AXIS1_FULL_INV".into(),
		]);
		assert_eq!(keys.keys.len(), 3);
	}

	#[test]
	#[should_panic(expected = "Incorrect keybinding JOY_WHEEL")]
	fn rejects_unknown_names() {
		parse_keybinding(vec!["JOY_WHEEL".into()]);
	}

	#[test]
	fn shipped_keyconfig_parses() {
		let text = include_str!("../dist/keyconfig.toml");
		// The live settings, plus the commented wheel example.
		let example: String = text
			.lines()
			.filter_map(|line| line.strip_prefix("# "))
			.filter(|line| line.contains(" = [\""))
			.map(|line| format!("{line}\n"))
			.collect();
		for source in [text, example.as_str()] {
			let table: BTreeMap<String, Vec<String>> = toml::from_str(source).unwrap();
			assert!(!table.is_empty());
			for (name, keys) in table {
				assert_eq!(
					parse_keybinding(keys.clone()).keys.len(),
					keys.len(),
					"{name}"
				);
			}
		}
	}

	/// Drives a virtual SDL wheel (4 axes, 8 buttons, 1 hat) through the real
	/// PollState: hot-plug, polling, pedal and steering mapping, tap/release.
	#[test]
	fn reads_a_virtual_wheel() {
		use sdl2::sys::*;
		let binding = |name: &str| parse_keybinding(vec![name.to_string()]);
		let steer_left = binding("JOY_AXIS0-");
		let steer_right = binding("JOY_AXIS0+");
		let gas = binding("JOY0_AXIS2_FULL_INV");
		let gear_up = binding("JOY0_BUTTON4");
		let hat_left = binding("JOY_HAT0_LEFT");

		let mut state = PollState::new(std::ptr::null_mut(), 0.01, true).unwrap();
		unsafe {
			let index =
				SDL_JoystickAttachVirtual(SDL_JoystickType::SDL_JOYSTICK_TYPE_WHEEL, 4, 8, 1);
			assert!(index >= 0, "virtual joystick not attached");
			let wheel = SDL_JoystickOpen(index);
			assert!(!wheel.is_null());

			// At rest: wheel centred, pedal up (+1).
			SDL_JoystickSetVirtualAxis(wheel, 0, 0);
			SDL_JoystickSetVirtualAxis(wheel, 2, i16::MAX);
			state.update();
			assert!(
				state.joys.iter().flatten().count() >= 1,
				"device not opened on hot-plug"
			);
			assert_eq!(state.is_down(&gas), 0.0);
			assert_eq!(state.is_down(&steer_right), 0.0);

			// Pedal floored, wheel half right, button and hat pressed.
			SDL_JoystickSetVirtualAxis(wheel, 0, i16::MAX / 2);
			SDL_JoystickSetVirtualAxis(wheel, 2, i16::MIN);
			SDL_JoystickSetVirtualButton(wheel, 4, 1);
			SDL_JoystickSetVirtualHat(wheel, 0, 8);
			state.update();
			assert_eq!(state.is_down(&gas), 1.0);
			assert!((state.is_down(&steer_right) - 0.5).abs() < 0.01);
			assert_eq!(state.is_down(&steer_left), 0.0);
			assert!(state.is_tapped(&gear_up));
			assert_eq!(state.is_down(&hat_left), 1.0);

			// Held: no longer a tap.
			state.update();
			assert!(!state.is_tapped(&gear_up));
			assert_eq!(state.is_down(&gear_up), 1.0);

			// Released.
			SDL_JoystickSetVirtualButton(wheel, 4, 0);
			state.update();
			assert!(state.is_released(&gear_up));

			// Unplugged: everything reads 0.
			SDL_JoystickClose(wheel);
			SDL_JoystickDetachVirtual(index);
			state.update();
			assert_eq!(state.is_down(&gas), 0.0);
			assert_eq!(state.is_down(&steer_right), 0.0);
		}
	}

	#[test]
	fn maps_axis_values() {
		let dz = 0.01;
		// Steering: centred is 0, each half goes 0..1.
		assert_eq!(axis_value(0.0, AxisMode::Positive, dz), 0.0);
		assert_eq!(axis_value(0.5, AxisMode::Positive, dz), 0.5);
		assert_eq!(axis_value(-0.5, AxisMode::Positive, dz), 0.0);
		assert_eq!(axis_value(-1.0, AxisMode::Negative, dz), 1.0);
		assert_eq!(axis_value(0.005, AxisMode::Positive, dz), 0.0);
		// Pedal resting at -1.
		assert_eq!(axis_value(-1.0, AxisMode::Full, dz), 0.0);
		assert_eq!(axis_value(0.0, AxisMode::Full, dz), 0.5);
		assert_eq!(axis_value(1.0, AxisMode::Full, dz), 1.0);
		// Pedal resting at +1.
		assert_eq!(axis_value(1.0, AxisMode::FullInverted, dz), 0.0);
		assert_eq!(axis_value(0.0, AxisMode::FullInverted, dz), 0.5);
		assert_eq!(axis_value(-1.0, AxisMode::FullInverted, dz), 1.0);
		assert_eq!(normalize(i16::MIN), -1.0);
		assert_eq!(normalize(i16::MAX), 1.0);
	}
}
