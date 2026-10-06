//! The magnetic card printer, emulated against `card.bin`.
//!
//! The RPS command set, the status-byte semantics, the `card.bin` layout and
//! the partial-write track selectors are ported from tattoohanz's C++ LINE
//! plugin (GPL-3), whose comments record how each was measured. Three of its
//! findings are the reason this module is not the six-command version it was:
//!
//! * `clCardPrinter::startSequence()` **zeroes s1/s2/s3 before every
//!   command**, and `0x00` is not `0x30`, so a command that answers without
//!   writing a status leaves `chkNoError()` false. `READSTATUS` -- the command
//!   the boot sequence relies on -- used to write no status at all, which is
//!   the intermittent "card reader not functional" on the title screen.
//! * **The game writes tracks individually.** `requestWrite()` diffs the new
//!   card against the old one per 69-byte track and sends only what changed,
//!   and the checksum lives in track 3. A partial write over a zero-filled
//!   buffer therefore produces a correctly sized card with dead tracks and a
//!   checksum that matches nothing -- and one `EJECT` followed by any partial
//!   write used to brick the saved card permanently.
//! * **An unknown command is a missing feature, not a reason to stop.** Five
//!   of the thirteen commands reached a `panic!` here, `PRINTSTRING` among
//!   them -- the mileage banner at the end of a race, i.e. on the one path
//!   nobody had run yet. With `panic = 'abort'` that was the process.
//!
//! `respond` is deliberately a pure function over the command, its payload and
//! the card buffer, so the whole state machine is unit-tested rather than
//! reasoned about.

use std::io::Write as _;
use std::sync::Mutex;

use crate::platform::call_game;
use crate::*;

const INIT: u8 = 0x10;
const READSTATUS: u8 = 0x20;
const CANCEL: u8 = 0x40;
const EJECT: u8 = 0x80;
const DISPENSECARD: u8 = 0xB0;
const PRINTSETTING: u8 = 0x78;
const READ: u8 = 0x33;
const WRITE: u8 = 0x53;
// The rest of the set, named after the `clCardPrinter::send*` member each
// comes from. Every one of these turns up in a session.
const EXTRACHAR: u8 = 0x7A; // sendExtraCharacterCommand
const PRINTSTRING: u8 = 0x7C; // sendStringCommand -- the mileage banner
const ERASE: u8 = 0x7D; // sendEraseCommand
const CLEANING: u8 = 0xA0; // sendCleaningCommand
const BATTERYCHECK: u8 = 0xF5; // sendBatteryCheckCommand

/// Three tracks of 69 bytes. `clV386CardData::assertion()` is `cmp ebx, 0xcf`.
const CARD_SIZE: usize = 0x45 * 3;
/// `clV386CardData::getSumStart()`: the checksum's own offset.
const CARD_SUM_AT: usize = 0xCD;

// The status bytes `clCardPrinter::getRPStoStatus` hands the game, read out of
// the printer object itself:
//
//   [this+0x04]  s1   0x30 no card, 0x31..0x33 card in
//   [this+0x05]  s2   0x30 normal; 0x31 and 0x51..0x56 read errors,
//                     0x40 a battery error
//   [this+0x06]  s3   0x30 normal; 0x32 "cannot execute", 0x35 empty dispenser
//   [this+0x1c]  signed; negative means CONNECT ERROR
//
// `clCardDeviceGameService::clStatus::setupResult()` walks every `chk*` in
// turn and only reaches result `0xC` -- the single value
// `clSeqBootCardDeviceThread` accepts -- when all of them are false and
// `chkNoCard` is true.
const ST_NO_CARD: u8 = 0x30;
const ST_CARD_IN: u8 = 0x31;
const ST_NORMAL: u8 = 0x30;
/// s3 for "no card to read".
const ST_NO_DATA: u8 = 0x34;

pub static mut CARD_DATA: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// What the printer answers, and what the caller must then do.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
	s1: u8,
	s2: u8,
	s3: u8,
	/// Copy the 207 card bytes to `write_buf + 6`.
	serve: bool,
	/// Flush the buffer to `card.bin`.
	save: bool,
}

impl Reply {
	/// Finished, with these three status bytes and nothing else to do.
	fn status(s1: u8, s2: u8, s3: u8) -> Self {
		Reply {
			s1,
			s2,
			s3,
			serve: false,
			save: false,
		}
	}

	/// Finished, healthy, nothing in the slot -- the answer to anything that
	/// needs no emulation.
	fn ok() -> Self {
		Reply::status(ST_NO_CARD, ST_NORMAL, ST_NORMAL)
	}
}

/// CRC-16/CCITT-FALSE: poly `0x1021`, init `0xFFFF`, no reflection, no final
/// xor. `clV386CardData` covers bytes `0..0xCD` and stores the result
/// little-endian at `0xCD`, i.e. inside track 3.
fn crc16(data: &[u8]) -> u16 {
	let mut crc = 0xFFFF_u16;
	for byte in data {
		crc ^= (*byte as u16) << 8;
		for _ in 0..8 {
			crc = if crc & 0x8000 != 0 {
				(crc << 1) ^ 0x1021
			} else {
				crc << 1
			};
		}
	}
	crc
}

/// Whether a buffer is a full card whose stored checksum matches its payload.
fn is_valid(card: &[u8]) -> bool {
	if card.len() != CARD_SIZE {
		return false;
	}
	let stored = u16::from_le_bytes([card[CARD_SUM_AT], card[CARD_SUM_AT + 1]]);
	stored == crc16(&card[..CARD_SUM_AT])
}

/// Decides the printer's answer. Pure: `card` is the in-memory card, `saved`
/// is whatever `card.bin` holds, and nothing here touches the disk.
fn respond(
	command: u8,
	payload: &[u8],
	card: &mut Vec<u8>,
	saved: &dyn Fn() -> Option<Vec<u8>>,
) -> Reply {
	match command {
		// The emulated reader is always healthy and always empty: nothing is
		// physically sitting in a slot, and a card only ever appears through
		// READ. Answering that deterministically is what lets setupResult()
		// settle on 0xC instead of on whatever initSerial() left behind.
		INIT | READSTATUS | CANCEL => Reply::ok(),

		// An empty payload is legal -- the command may carry none at all.
		DISPENSECARD => {
			if payload.first() == Some(&0x32) {
				Reply::status(ST_NO_CARD, ST_NORMAL, 0x37)
			} else {
				Reply::status(0x33, ST_NORMAL, ST_NORMAL)
			}
		}

		READ => {
			// `payload[0] == 0x32` is a presence query, not a read.
			if payload.first() == Some(&0x32) {
				if card.is_empty() {
					return Reply::status(ST_NO_CARD, ST_NORMAL, ST_NO_DATA);
				}
				return Reply::status(ST_CARD_IN, ST_NORMAL, ST_NORMAL);
			}
			if card.len() < CARD_SIZE {
				// Report an empty reader rather than read out of bounds.
				return Reply::status(ST_NO_CARD, ST_NORMAL, ST_NO_DATA);
			}
			Reply {
				serve: true,
				..Reply::status(ST_CARD_IN, ST_NORMAL, ST_NORMAL)
			}
		}

		// `payload[0..3]` is a header whose third byte selects the tracks;
		// the track data follows it.
		WRITE => {
			let reply = Reply::status(ST_CARD_IN, ST_NORMAL, ST_NORMAL);
			let Some(&selector) = payload.get(2) else {
				report(format!(
					"card: WRITE with no track selector ({} payload bytes), ignored",
					payload.len()
				));
				return reply;
			};
			// An indexed write only touches one track, so whatever backs the
			// others should be real data: prefer the saved card over zeroes.
			if card.len() < CARD_SIZE {
				*card = saved().unwrap_or_default();
				card.resize(CARD_SIZE, 0);
			}
			let data = &payload[3..];
			let (offset, length) = match selector {
				0x30 => (0, 0x45),        // track 1
				0x35 => (0x45, 0x45 * 2), // tracks 2 and 3
				0x36 => (0, CARD_SIZE),   // all three
				_ => {
					report(format!(
						"card: WRITE with unknown track selector {selector:#04x}, ignored"
					));
					return reply;
				}
			};
			if data.len() < length {
				report(format!(
					"card: short WRITE for selector {selector:#04x}: {} of {length} bytes, \
					 ignored",
					data.len()
				));
				return reply;
			}
			card[offset..offset + length].copy_from_slice(&data[..length]);
			Reply {
				save: true,
				..reply
			}
		}

		// A real reader spits the card out and the player pushes it back in.
		// There is no insert command here, so dropping the buffer left the
		// next READ with nothing -- which the game reports as "Card data load
		// error" -- and the next partial write building on 207 zeroes.
		EJECT => {
			*card = saved().unwrap_or_default();
			if !card.is_empty() && card.len() < CARD_SIZE {
				card.resize(CARD_SIZE, 0);
			}
			Reply::ok()
		}

		// Nothing to emulate: there is no thermal head, no ribbon and no
		// battery. The print state machine
		// (`clCardDeviceGameService::clPrintProcess::run`) only polls
		// `chkFinish()` and `chkNoError()` -- it wants the command word back
		// at 0, s2 == 0x30, s3 != 0x32 and a non-negative `[this+0x1c]`.
		// BATTERYCHECK in particular must not answer 0x40 in s2, which is how
		// a real reader reports a flat backup battery.
		PRINTSETTING | EXTRACHAR | ERASE | CLEANING | BATTERYCHECK | PRINTSTRING => Reply::ok(),

		// An unknown command is a missing feature, not a reason to take the
		// process down mid-race. Answering "finished, no error" keeps the
		// state machine moving and the report says what turned up.
		_ => {
			report(format!(
				"card: unhandled command {command:#04x} ({} payload bytes), answering OK",
				payload.len()
			));
			Reply::ok()
		}
	}
}

/// Says something once. The printer is polled, so a command that keeps
/// arriving would otherwise fill the log with the same line.
fn report(message: String) {
	static REPORTED: report::Reported = report::Reported::new();
	REPORTED.say(message);
}

/// Either nothing, or exactly one card: a short buffer is padded and a long
/// one cut.
///
/// Both halves matter because `save_card_file` writes only an exactly
/// 207-byte buffer. A `card.bin` that is some other length -- appended to, or
/// written by something with a different idea of the format -- would otherwise
/// load and serve happily while every save for the rest of the session was
/// refused with nothing but one line to say why.
fn one_card(mut card: Vec<u8>) -> Vec<u8> {
	if card.is_empty() || card.len() == CARD_SIZE {
		return card;
	}
	report(format!(
		"card: card.bin is {} bytes, using {CARD_SIZE} of it",
		card.len()
	));
	card.resize(CARD_SIZE, 0);
	card
}

/// Reads `card.bin`. `None` if there isn't one.
fn read_card_file() -> Option<Vec<u8>> {
	Some(one_card(std::fs::read("card.bin").ok()?))
}

/// Loads `card.bin` into the reader. `what` names the occasion, since this is
/// both the boot load and the CARD_INSERT binding; the message is not
/// deduplicated because a player pressing the key wants an answer every time.
unsafe fn load_card(what: &str) {
	match read_card_file() {
		Some(card) => {
			platform::log(&format!(
				"card: {what} card.bin, {} bytes, checksum {}",
				card.len(),
				if is_valid(&card) {
					"matches"
				} else {
					"does not match"
				}
			));
			*CARD_DATA.lock().unwrap_or_else(|error| error.into_inner()) = card;
		}
		None => platform::log(&format!("card: no card.bin to {what}")),
	}
}

/// Puts the card back in the reader, for the CARD_INSERT binding. The printer
/// answers READ out of this buffer, so this is the same lock it takes.
pub unsafe fn insert() {
	load_card("inserted");
}

/// Flushes the buffer, but only when it is a whole card. Refusing a short one
/// is always safer than overwriting a good save with a half-populated buffer.
///
/// A checksum that does not match is reported and **still written**. The
/// tempting thing is to refuse, but the checksum covers bytes `0..0xCD`, which
/// spans all three tracks, so a track-1 write ought to change it -- and the
/// game sends track 1 alone anyway. Either our idea of the wire format is
/// incomplete or the game keeps the sum some other way, and until that is
/// measured, a mismatch is not evidence that these 207 bytes are wrong. The
/// bytes are the game's own, and the failure modes are not symmetric: writing
/// them risks a card the game will reject, while refusing silently throws away
/// a race the player just finished.
unsafe fn save_card_file(card: &[u8]) {
	if card.len() != CARD_SIZE {
		report(format!(
			"card: not saving, the buffer is {} bytes rather than {CARD_SIZE}",
			card.len()
		));
		return;
	}
	if !is_valid(card) {
		report(String::from(
			"card: the checksum at 0xCD does not match the payload, saving anyway -- if the \
			 game reports a card error after this, that assumption is what to look at",
		));
	}
	// Write beside the real file and rename over it, so a crash mid-write
	// cannot leave a truncated card where a good one was.
	let temporary = "card.bin.new";
	let written = std::fs::File::create(temporary)
		.and_then(|mut file| file.write_all(card).and_then(|()| file.sync_all()));
	if let Err(error) = written.and_then(|()| std::fs::rename(temporary, "card.bin")) {
		platform::log(&format!("card: cannot save card.bin: {error}"));
		_ = std::fs::remove_file(temporary);
		return;
	}
	if CONFIG.card_log {
		platform::log("card: saved card.bin");
	}
}

/// `clCardPrinter::exec`, replaced outright. `printer` is the printer object:
/// the pending-command word is its first dword, and writing 0 there is how the
/// game is told the command finished.
unsafe extern "C" fn exec(printer: *mut u32) {
	if printer.read() == 0 {
		return;
	}
	let request = printer.byte_add(0x08).read() as *const u8;
	if request.is_null() || request.read() != 2 {
		return;
	}
	let count = request.byte_add(1).read();
	let command = request.byte_add(2).read();
	let mut payload = Vec::new();
	for i in 6..count {
		payload.push(request.byte_add(i as usize).read());
	}

	let (reply, to_save) = {
		let mut card = CARD_DATA.lock().unwrap_or_else(|error| error.into_inner());
		let reply = respond(command, &payload, &mut card, &read_card_file);
		if CONFIG.card_log {
			platform::log(&format!(
				"card: {} ({command:#04x}), {} payload bytes, buffer {} -> \
				 s1={:02x} s2={:02x} s3={:02x}{}{}",
				command_name(command),
				payload.len(),
				card.len(),
				reply.s1,
				reply.s2,
				reply.s3,
				if reply.serve {
					", serving the card"
				} else {
					""
				},
				if reply.save { ", saving" } else { "" },
			));
		}
		if reply.serve {
			// `clCardPrinter::getReadData()` is
			// `memcpy(dst, this[0x10] + 6, 0xcf)`, so the game expects the 207
			// card bytes to start six into the response buffer.
			let write_buf = printer.byte_add(0x10).read() as *mut u8;
			if !write_buf.is_null() {
				write_buf.write(0x00);
				write_buf.byte_add(0x04).write(0x33);
				write_buf.byte_add(0x05).write(0x30);
				std::ptr::copy_nonoverlapping(card.as_ptr(), write_buf.byte_add(6), CARD_SIZE);
			}
		}
		// Taken out from under the lock, so the disk write below does not
		// block whoever else wants the card.
		let to_save = reply.save.then(|| card.clone());
		(reply, to_save)
	};
	if let Some(card) = to_save {
		save_card_file(&card);
	}

	// The status before the command word, because the command word is what the
	// game polls to decide the status is worth reading: `startSequence()`
	// zeroes s1/s2/s3 before every command, and a reader that saw the 0 here
	// first would read those zeroes, fail `chkNoError()` and draw the card
	// error. The connect error goes with them -- a failed `initSerial()`
	// latches it, and `chkNoError()` wants it non-negative.
	let status = printer as *mut u8;
	status.byte_add(0x04).write(reply.s1);
	status.byte_add(0x05).write(reply.s2);
	status.byte_add(0x06).write(reply.s3);
	(status.byte_add(0x1c) as *mut i32).write(0);
	printer.write(0);
}

fn command_name(command: u8) -> &'static str {
	match command {
		INIT => "INIT",
		READSTATUS => "READSTATUS",
		CANCEL => "CANCEL",
		EJECT => "EJECT",
		DISPENSECARD => "DISPENSE",
		PRINTSETTING => "PRINTSET",
		EXTRACHAR => "EXTRACHAR",
		PRINTSTRING => "PRINTSTR",
		ERASE => "ERASE",
		CLEANING => "CLEANING",
		BATTERYCHECK => "BATTCHECK",
		READ => "READ",
		WRITE => "WRITE",
		_ => "unknown",
	}
}

/// What `clCardDeviceGameService::getResult` means, from the messages each
/// code puts on screen.
fn result_name(result: c_int) -> &'static str {
	match result {
		0 => "idle",
		1 => "busy",
		2 => "dispenser empty",
		3 => "connect -- check card R/W connections",
		4 => "write error -- data save failure",
		5 => "read error -- card only partially read",
		6 => "block error -- card jam",
		7 => "print error",
		8 => "system error",
		9 => "illegal error -- card jam",
		0xA => "cannot execute -- card ejection error",
		0xB => "battery backup voltage too low",
		0xC => "no card, no error",
		_ => "unknown",
	}
}

/// 3DX and 3DX+ added a `clCardDeviceGameAutoService` that polls the card
/// device every frame. `getResult` answering 0 or 0xC is OK and 1 is pending;
/// anything else latches a hard error, which calls `clNet::stop()` and draws
/// `clMsgCardDeviseError`. With no real device answering, an idle attract loop
/// eventually gets there -- so report OK and it never latches.
unsafe extern "C" fn auto_service_result(_this: *const c_void) -> c_int {
	0
}

static mut ORIGINAL_SERVICE_RESULT: *const () = std::ptr::null();
/// The getter `clSeqBootCardDeviceThread::run` reads: `<= 1` means still
/// working, `0xC` means OK, anything else draws the error. With the status
/// bytes answered properly this should settle on `0xC` by itself, so this only
/// says when it does not.
unsafe extern "C" fn service_result(this: *const c_void) -> c_int {
	let result = call_game(ORIGINAL_SERVICE_RESULT, (this,));
	if result != 0 && result != 1 && result != 0xC {
		report(format!(
			"card: clCardDeviceGameService::getResult says {result} ({}), which draws the \
			 card error",
			result_name(result)
		));
	}
	result
}

pub unsafe fn init() {
	// A saved card up front, so a READ right after boot has something to serve.
	load_card("loaded");

	hook::hook_symbol("_ZN13clCardPrinter4execEv", exec as *const ());

	// Absent on base WMMT3, which has no such class.
	let auto_service = hook::get_symbol("_ZNK27clCardDeviceGameAutoService9getResultEv");
	if auto_service.is_null() {
		platform::log("card: no clCardDeviceGameAutoService, nothing to keep quiet");
	} else {
		hook::hook(auto_service, auto_service_result as *const ());
	}

	// Const on 3DX/3DX+, non-const on base WMMT3: take whichever resolves.
	let mut service = hook::get_symbol("_ZNK23clCardDeviceGameService9getResultEv");
	if service.is_null() {
		service = hook::get_symbol("_ZN23clCardDeviceGameService9getResultEv");
	}
	if service.is_null() {
		platform::log("card: clCardDeviceGameService::getResult is missing");
	} else {
		ORIGINAL_SERVICE_RESULT = hook::hook(service, service_result as *const ());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A card whose checksum matches, with each track filled with its own
	/// number so a partial write is visible.
	fn card() -> Vec<u8> {
		let mut card = vec![0_u8; CARD_SIZE];
		card[..0x45].fill(1);
		card[0x45..0x45 * 2].fill(2);
		card[0x45 * 2..].fill(3);
		let sum = crc16(&card[..CARD_SUM_AT]).to_le_bytes();
		card[CARD_SUM_AT] = sum[0];
		card[CARD_SUM_AT + 1] = sum[1];
		card
	}

	fn nothing_saved() -> Option<Vec<u8>> {
		None
	}

	/// The published CCITT-FALSE check value.
	#[test]
	fn crc_matches_the_check_vector() {
		assert_eq!(crc16(b"123456789"), 0x29B1);
		assert_eq!(crc16(&[]), 0xFFFF);
	}

	#[test]
	fn validates_a_whole_card_only() {
		assert!(is_valid(&card()));
		let mut wrong = card();
		wrong[0] = 0xFF;
		assert!(!is_valid(&wrong));
		assert!(!is_valid(&card()[..CARD_SIZE - 1]));
		assert!(!is_valid(&[]));
	}

	/// Every command answers s2 == 0x30 and s3 != 0x32, which is all
	/// `chkNoError()` wants, and none of them is allowed to be a panic.
	#[test]
	fn every_command_answers_without_an_error() {
		let commands = [
			INIT,
			READSTATUS,
			CANCEL,
			EJECT,
			DISPENSECARD,
			PRINTSETTING,
			EXTRACHAR,
			PRINTSTRING,
			ERASE,
			CLEANING,
			BATTERYCHECK,
			READ,
			WRITE,
			0x99, // and one that does not exist
		];
		for command in commands {
			let mut buffer = card();
			let reply = respond(command, &[], &mut buffer, &nothing_saved);
			assert_eq!(
				reply.s2, ST_NORMAL,
				"command {command:#04x} reports an error"
			);
			assert_ne!(reply.s3, 0x32, "command {command:#04x} cannot execute");
		}
	}

	/// BATTERYCHECK answering 0x40 in s2 is how a real reader reports a flat
	/// backup battery.
	#[test]
	fn battery_check_reports_a_healthy_battery() {
		let mut buffer = card();
		assert_eq!(
			respond(BATTERYCHECK, &[], &mut buffer, &nothing_saved).s2,
			0x30
		);
	}

	#[test]
	fn reads_a_whole_card_or_says_there_is_none() {
		let mut buffer = card();
		let reply = respond(READ, &[0x00], &mut buffer, &nothing_saved);
		assert!(reply.serve);
		assert_eq!(reply.s1, ST_CARD_IN);

		// A presence query serves nothing.
		let reply = respond(READ, &[0x32], &mut buffer, &nothing_saved);
		assert!(!reply.serve);
		assert_eq!(reply.s1, ST_CARD_IN);

		// Nothing to read reports an empty reader rather than reading short.
		let mut empty = Vec::new();
		let reply = respond(READ, &[0x00], &mut empty, &nothing_saved);
		assert!(!reply.serve);
		assert_eq!((reply.s1, reply.s3), (ST_NO_CARD, ST_NO_DATA));
		let mut short = vec![0_u8; 0x45];
		assert!(!respond(READ, &[0x00], &mut short, &nothing_saved).serve);
	}

	/// A write command: two header bytes, the track selector, then `length`
	/// bytes of track data, all 9s so a misplaced write is obvious.
	fn write_command(selector: u8, length: usize) -> Vec<u8> {
		let mut payload = vec![0x00, 0x00, selector];
		payload.resize(3 + length, 9);
		payload
	}

	#[test]
	fn writes_each_track_selection_at_its_own_offset() {
		let mut buffer = card();
		assert!(
			respond(
				WRITE,
				&write_command(0x30, 0x45),
				&mut buffer,
				&nothing_saved
			)
			.save
		);
		assert_eq!(buffer[0], 9);
		assert_eq!(buffer[0x45], 2, "track 1 write touched track 2");

		let mut buffer = card();
		assert!(
			respond(
				WRITE,
				&write_command(0x35, 0x45 * 2),
				&mut buffer,
				&nothing_saved
			)
			.save
		);
		assert_eq!(buffer[0], 1, "track 2+3 write touched track 1");
		assert_eq!(buffer[0x45], 9);
		assert_eq!(buffer[CARD_SIZE - 1], 9);

		let mut buffer = card();
		assert!(
			respond(
				WRITE,
				&write_command(0x36, CARD_SIZE),
				&mut buffer,
				&nothing_saved
			)
			.save
		);
		assert_eq!(buffer, vec![9_u8; CARD_SIZE]);
	}

	/// A write that cannot be honoured leaves the card alone and still answers
	/// without an error, because the alternative was a corrupt save.
	#[test]
	fn refuses_a_write_it_cannot_honour() {
		for payload in [
			vec![],
			vec![0, 0],
			vec![0, 0, 0x30, 1, 2, 3],
			vec![0, 0, 0x99],
		] {
			let mut buffer = card();
			let reply = respond(WRITE, &payload, &mut buffer, &nothing_saved);
			assert!(!reply.save, "saved a card built from {payload:?}");
			assert_eq!(buffer, card(), "corrupted the card from {payload:?}");
			assert_eq!(reply.s2, ST_NORMAL);
		}
	}

	/// The bricking sequence: EJECT used to clear the buffer, and the next
	/// partial write then built a full-sized card out of one real track and
	/// 138 zeroes -- with a checksum covering none of it.
	#[test]
	fn eject_then_a_partial_write_keeps_the_other_tracks() {
		let saved = || Some(card());
		let mut buffer = card();

		respond(EJECT, &[], &mut buffer, &saved);
		assert_eq!(
			buffer,
			card(),
			"EJECT dropped the card instead of reloading it"
		);

		let payload = write_command(0x30, 0x45);
		let reply = respond(WRITE, &payload, &mut buffer, &saved);
		assert!(reply.save);
		assert_eq!(buffer[0x45], 2, "tracks 2 and 3 were lost");
		assert_eq!(buffer[0x45 * 2], 3);

		// And with nothing on disk at all, a partial write still produces a
		// full-sized buffer rather than panicking on an empty one.
		let mut nothing = Vec::new();
		assert!(respond(WRITE, &payload, &mut nothing, &nothing_saved).save);
		assert_eq!(nothing.len(), CARD_SIZE);
	}

	/// Whatever `card.bin` holds, what the rest of the module sees is nothing
	/// or exactly one card -- otherwise it loads and serves while every save
	/// is refused for being the wrong length.
	#[test]
	fn a_card_file_is_padded_or_cut_to_one_card() {
		assert!(
			one_card(Vec::new()).is_empty(),
			"invented a card from nothing"
		);
		assert_eq!(one_card(vec![1; CARD_SIZE]), vec![1; CARD_SIZE]);

		let short = one_card(vec![1; 0x45]);
		assert_eq!(short.len(), CARD_SIZE);
		assert_eq!(&short[..0x45], &[1; 0x45], "padding overwrote track 1");
		assert_eq!(short[0x45], 0);

		let long = one_card(vec![1; CARD_SIZE + 93]);
		assert_eq!(long.len(), CARD_SIZE);
		assert!(long.iter().all(|byte| *byte == 1), "cut the wrong end off");
	}

	/// A partial write onto an empty buffer still ends up a whole card, so the
	/// save that follows it is not refused for its length.
	#[test]
	fn a_partial_write_onto_nothing_still_makes_one_card() {
		let mut buffer = Vec::new();
		assert!(
			respond(
				WRITE,
				&write_command(0x30, 0x45),
				&mut buffer,
				&nothing_saved
			)
			.save
		);
		assert_eq!(buffer.len(), CARD_SIZE);
	}

	#[test]
	fn eject_with_no_saved_card_reports_an_empty_reader() {
		let mut buffer = card();
		let reply = respond(EJECT, &[], &mut buffer, &nothing_saved);
		assert!(buffer.is_empty());
		assert_eq!(reply.s1, ST_NO_CARD);
	}

	#[test]
	fn dispense_distinguishes_its_query() {
		let mut buffer = card();
		assert_eq!(
			respond(DISPENSECARD, &[0x32], &mut buffer, &nothing_saved).s3,
			0x37
		);
		assert_eq!(
			respond(DISPENSECARD, &[], &mut buffer, &nothing_saved).s1,
			0x33
		);
	}
}
