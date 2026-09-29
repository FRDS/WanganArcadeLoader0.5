# Plan: controller input you can test (pads + steering wheels)

(The Windows port plan, phases P0–P7, is in progress. This covers the input part of P3, brought forward. P0–P2 are committed locally as `1da366d` and `c52af67`.)

## Context

The first Windows build runs, and the user tested it, but the only input they found was F1. The default `keyconfig.toml` (identical in WAL, LINE's C++ port and tattoohanz's pack) binds F1 = TEST and F2 = SERVICE. Everything else is on the gamepad, and nothing tells the user that. The gaps they're hitting:

1. **No keyboard driving keys** in the defaults (gas, brake, steering, gears, view).
2. **Steering wheels and pedals** usually aren't in SDL's gamepad database. `poll.rs` only understands SDL *GameController* buttons and axes, so a wheel is ignored.
3. **No visibility.** Nothing logs which controllers SDL found or what they send, so a failing pad can't be told apart from a missing binding.
4. **Robustness bugs** in `poll.rs`:
   - Hot-plug keys controllers by device index but removes them by instance id, so removal never matches.
   - `gamepad.open(which).unwrap()` panics on hot-plug if a device won't open as a controller.
   - A missing `gamecontrollerdb.txt` fails `PollState::new`, and the `unwrap()` in `jamma.rs` turns that into a crash.

The user will test with an Xbox/PlayStation pad and a steering wheel with pedals.

## Changes

**`src/poll.rs`**
- **Raw joystick bindings**, parsed in `parse_keybinding` next to the existing `MAPPINGS` table:
  - `JOY_BUTTON<n>`
  - `JOY_AXIS<n>+` / `JOY_AXIS<n>-` (half axis, for steering)
  - `JOY_AXIS<n>_FULL` / `JOY_AXIS<n>_FULL_INV` (whole travel mapped to 0..1, for pedals that rest at one end)
  - `JOY_HAT<n>_UP/DOWN/LEFT/RIGHT`

  Each can be limited to one device with a device prefix (`JOY1_AXIS2_FULL_INV`); without one, it matches any device. New `KeyBinding::JoyAxis` / `JoyButton` / `JoyHat` variants plug into the existing `binding_is_down` / `was_down` / `is_tapped` helpers, so `jamma.rs` needs no changes.
- **Every attached joystick is opened** through SDL's joystick subsystem, and hot-plug follows `JoyDeviceAdded` / `JoyDeviceRemoved`. Its axes, buttons and hats are **polled** each update after the event pump. Polling means pedals read correctly before they're first moved.
- **Deadzone** applies to raw axes, the same as it does for pad axes.
- **Fixes:**
  - controllers keyed by `instance_id()`;
  - no `unwrap` on hot-plug;
  - a missing `gamecontrollerdb.txt` becomes a warning, and SDL's built-in pad mappings still apply;
  - set `SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS=1` before init, so input doesn't depend on SDL's own idea of focus.

**`input_log = true`** in `config.toml` (a new `Config` field, default off). It writes to `wal_3dxp.log` on Windows and stdout on Linux, through a small `platform::log()`:
- at start: every device, with its index, name, whether SDL sees it as a gamepad, and its axis, button and hat counts;
- while playing: each button and hat change, and each axis crossing ±50%, with the binding name to use (e.g. `JOY0_AXIS2 = -0.98 (pedal? try JOY0_AXIS2_FULL_INV)`).

This is how the user finds their wheel's axis numbers.

**`dist/keyconfig.toml`** (shared by Linux and Windows; only keyboard keys and comments are added):
- **Keyboard:**
  - W gas, S brake, A/D steer
  - E/Q gear up/down, NUM0–NUM6 neutral/1st–6th
  - V view, SPACE intrude
  - C card, F1 test, F2 service
  - the arrow keys stay on the debug-menu gear bindings
- **Pad:** the defaults stay as they are.
- **Commented wheel example:** steering `JOY0_AXIS0-`/`+`, pedals `JOY0_AXIS1_FULL_INV`-style, with a note to confirm the numbers with `input_log`.

**`dist-windows/README.md`**: a table of the default controls (keyboard and pad), and wheel setup in three steps: turn on `input_log`, press or turn each control, then copy the logged names into `keyconfig.toml`.

## Verification

- **Unit tests** in `poll.rs` (`cargo test --lib`, run natively for i586 and under Wine for Windows):
  - every new binding string parses to the right variant, and bad strings are rejected with the key name;
  - the axis mapping maths: half, full and inverted, with the deadzone, at the rest, middle and full positions.
- Both targets build, clippy shows no new warnings, and the mock-LINE test still passes (input isn't reached there).
- **User on Windows:**
  - F1/F2 plus W/A/S/D work in-game;
  - the pad drives, following the defaults;
  - with `input_log = true`, `wal_3dxp.log` lists the wheel and shows its axes moving;
  - after putting those names into `keyconfig.toml`, steering and pedals drive the car.
