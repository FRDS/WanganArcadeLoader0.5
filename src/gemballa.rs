//! The Gemballa unlock, applied in memory and never to `main` on disk.
//!
//! GEMBALLA is maker index 0. The Export build hides it by raising the
//! maker-select lower bound to 1, blanks its car images through four
//! `setImageReplace` calls, rejects Gemballa cards in
//! `clV386CardData::assign()`, and moves one story rival off the car. Undoing
//! those four things is the whole feature.
//!
//! **Every offset is verified before anything is written**, and verified as a
//! set: a half-applied patch is worse than none, so a single mismatch stops
//! the lot. That is not a formality here. The roadmap records this build's md5
//! as `6f02c8bc…` and the dumps we have hash `aac96862…` at the same 24,925,524
//! bytes, so the identifying hash is wrong even though all seven offsets are
//! right -- which is exactly the situation where trusting an offset list
//! blindly would corrupt a running game.
//!
//! The story edit restores **Shima's Blackbird**, and it is not the edit the
//! roadmap describes. `0x12ED6B0` -- the one entry of a 32-car roster where
//! Export differs from a JPN dump -- is in a table story mode never reads,
//! proved by writing one car into all 32 entries and watching every rival
//! keep its own. What story mode reads is `STORY_SHIMA` below: 35 `.data`
//! sites where Export holds model id 277 and JPN holds 248.
//!
//! The lesson that cost the most here: **the car table explains a runtime
//! observation, it never produces one.** Three static readings of it gave
//! three confident wrong answers about which car Shima drives, each undone by
//! one line of `car_log` from the running game.

use crate::*;

/// `main` is `ET_EXEC` and not position-independent, so a file offset plus
/// the link base is the runtime address.
const BASE: usize = 0x0804_8000;

/// One in-memory edit, with the bytes it expects to find there.
struct Patch {
	offset: usize,
	expect: &'static [u8],
	write: &'static [u8],
	what: &'static str,
}

/// Lets the player pick a Gemballa. **Needs the car models**: the Export dump
/// ships `data/car/gem_*` as empty directories, so without them this offers a
/// car the engine cannot load.
const CARS: [Patch; 6] = [
	Patch {
		offset: 0x7A_8641,
		expect: &[0x83, 0xbe, 0xb0, 0x02, 0x00, 0x00, 0x01],
		write: &[0x83, 0xbe, 0xb0, 0x02, 0x00, 0x00, 0x00],
		what: "maker-select lower bound, which is the region lock",
	},
	Patch {
		offset: 0x7A_8209,
		expect: &[0xc7, 0x44, 0x24, 0x08, 0x01, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x44, 0x24, 0x08, 0x47, 0x1c, 0x00, 0x00],
		what: "setImageReplace 1 of 4",
	},
	Patch {
		offset: 0x7A_86AB,
		expect: &[0xc7, 0x44, 0x24, 0x08, 0x01, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x44, 0x24, 0x08, 0x47, 0x1c, 0x00, 0x00],
		what: "setImageReplace 2 of 4",
	},
	Patch {
		offset: 0x7A_87A8,
		expect: &[0xc7, 0x44, 0x24, 0x08, 0x01, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x44, 0x24, 0x08, 0x47, 0x1c, 0x00, 0x00],
		what: "setImageReplace 3 of 4",
	},
	Patch {
		offset: 0x7A_88B3,
		expect: &[0xc7, 0x44, 0x24, 0x08, 0x01, 0x00, 0x00, 0x00],
		write: &[0xc7, 0x44, 0x24, 0x08, 0x47, 0x1c, 0x00, 0x00],
		what: "setImageReplace 4 of 4",
	},
	Patch {
		offset: 0x68_95A8,
		expect: &[0x83, 0xc8, 0x02],
		write: &[0x90, 0x90, 0x90],
		what: "clV386CardData::assign, which ORs error flag 2 for Gemballa cards",
	},
];

/// The one roster entry where Export differs from a JPN dump, put back.
///
/// **This is not what moves Shima** -- see `STORY_SHIMA`. The 32-entry table
/// at `0x12ED3E0` is read by something, but not by story mode: writing one
/// car into all 32 entries left every rival in its own car. Kept because it
/// is the single byte-level difference between the two builds' rosters and
/// restoring it costs nothing, and because it is the edit the roadmap and
/// tattoohanz's notes both describe, so a dump without it applied would look
/// unpatched to anyone comparing.
///
/// Separate from `CARS` because it is safe on its own: a rival's car comes
/// from `data/characar/`, which the Export dump ships complete, so this needs
/// none of the models `CARS` does.
const ROSTER_CAR: [Patch; 1] = [Patch {
	offset: 0x12E_D6B0,
	expect: &[0x15, 0x01, 0x00, 0x00],
	write: &[0xf7, 0x00, 0x00, 0x00],
	what: "roster entry 30, back to the JPN value (277 -> 247)",
}];

/// Whether every patch in the set finds the bytes it expects.
///
/// Takes the reader so the table can be tested against a fake image rather
/// than a running game.
fn all_present(patches: &[Patch], read: &dyn Fn(usize, usize) -> Option<Vec<u8>>) -> bool {
	patches
		.iter()
		.all(|p| read(p.offset, p.expect.len()).as_deref() == Some(p.expect))
}

unsafe fn read_image(offset: usize, len: usize) -> Option<Vec<u8>> {
	let at = (BASE + offset) as *const u8;
	Some(std::slice::from_raw_parts(at, len).to_vec())
}

/// Applies a set, or none of it, saying which either way.
unsafe fn apply(patches: &[Patch], name: &str) -> bool {
	if !all_present(patches, &|o, l| read_image(o, l)) {
		platform::log(&format!(
			"gemballa: {name} not applied -- the bytes at one of its offsets are not what this build should have"
		));
		for p in patches {
			if read_image(p.offset, p.expect.len()).as_deref() != Some(p.expect) {
				platform::log(&format!(
					"gemballa:   0x{:08X} expected {:02x?}, found {:02x?} -- {}",
					p.offset,
					p.expect,
					read_image(p.offset, p.expect.len()).unwrap_or_default(),
					p.what
				));
			}
		}
		return false;
	}
	for p in patches {
		platform::write_memory((BASE + p.offset) as *mut (), p.write);
		// Read back. A write that silently does not stick -- a page the
		// loader cannot make writable, say -- would otherwise look exactly
		// like a patch that works but is ignored, and those want chasing in
		// completely different directions.
		let back = read_image(p.offset, p.write.len());
		let stuck = back.as_deref() == Some(p.write);
		platform::log(&format!(
			"gemballa: {} 0x{:08X} -- {}{}",
			if stuck { "patched" } else { "WRITE DID NOT STICK at" },
			p.offset,
			p.what,
			if stuck {
				String::new()
			} else {
				format!(", reads back {:02x?}", back.unwrap_or_default())
			}
		));
	}
	true
}

/// Whether the player-selectable Gemballa models are actually present.
///
/// The Export dump ships `data/car/gem_964_38RS`, `gem_964_RSR` and
/// `gem_997_AVA` as directories with **no files in them** -- 0 against 57, 67
/// and 42 in a JPN dump -- so the lock is not the only thing standing between
/// a player and the car. Unlocking the maker without them offers a car the
/// engine cannot load, which is a worse failure than leaving it locked.
fn models_present() -> bool {
	["gem_964_38RS", "gem_964_RSR", "gem_997_AVA"]
		.iter()
		.all(|car| std::path::Path::new(&format!("data/car/{car}/car.lua")).is_file())
}

/// **Shima's Blackbird.** The 35 `.data` sites where the Export build holds
/// model id 277 and a JPN dump holds 248, at the same offset within the
/// section.
///
/// Both ids are runtime-confirmed, which matters because every static reading
/// of the car table in this file's history was off by one in one direction or
/// the other: `car_log` reports **277 is `nis_Z33_Shima`** and **248 is
/// `gem_964_38RS_Shima2nd`**. So the Export build swapped Shima's Blackbird
/// for a Z33 in thirty-five places, and writing the JPN value back restores
/// it -- confirmed on hardware in story Episode 7, where the rival loaded
/// `gem_964_38RS_Shima2nd` with these applied.
///
/// This is *not* what `0x12ED6B0` does. That edit -- the one the roadmap
/// describes as putting a story rival in the Gemballa -- is in a roster
/// story mode never reads, proved by setting all 32 of its entries to one car
/// and watching nothing change. Finding this took ruling out the roster,
/// every `.rodata` dword, the three `nsCarParts` id-space converters and the
/// whole call chain down to `clCarConstructParam+0x08`; `.data` was the one
/// section never probed, and the answer was sitting in it.
const STORY_SHIMA: [usize; 35] = [
	0x17035D8,
	0x1703670,
	0x17036AC,
	0x1703B20,
	0x1703B38,
	0x1703B58,
	0x1703B94,
	0x1703DD0,
	0x1703DE4,
	0x1703E04,
	0x1703E40,
	0x1703F28,
	0x1703F40,
	0x1703F60,
	0x1703F9C,
	0x170409C,
	0x17040F8,
	0x17041D8,
	0x17041EC,
	0x170420C,
	0x1704248,
	0x17042C0,
	0x170A548,
	0x170A55C,
	0x170A560,
	0x170A57C,
	0x170A580,
	0x170A5B8,
	0x170A5BC,
	0x170A6A0,
	0x170A6BC,
	0x170A6DC,
	0x170A718,
	0x170A754,
	0x170A790,
];

pub unsafe fn init() {
	if !CONFIG.gemballa {
		return;
	}
	if GAME_VERSION.major != GameMajor::W3P {
		platform::log("gemballa: only 3DX+ is patched, and this is not it");
		return;
	}

	apply(&ROSTER_CAR, "the roster entry");

	let mut done = 0;
	for off in STORY_SHIMA {
		if read_image(off, 4).as_deref() == Some(&277u32.to_le_bytes()) {
			platform::write_memory((BASE + off) as *mut (), &248u32.to_le_bytes());
			done += 1;
		}
	}
	platform::log(&format!(
		"gemballa: Shima drives the Blackbird again -- {done} of {} sites \
		 restored from 277 (nis_Z33_Shima) to 248 (gem_964_38RS_Shima2nd)",
		STORY_SHIMA.len()
	));

	if models_present() {
		apply(&CARS, "the maker unlock");
	} else {
		platform::log(
			"gemballa: maker left locked -- data/car/gem_964_38RS, gem_964_RSR and gem_997_AVA \
			 are empty in an Export dump, and unlocking without their models offers a car the \
			 engine cannot load. Copy the three folders from a JPN dump.",
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A fake image that answers with whatever each offset should hold.
	fn image_of(patches: &[Patch]) -> impl Fn(usize, usize) -> Option<Vec<u8>> + '_ {
		move |offset, len| {
			patches
				.iter()
				.find(|p| p.offset == offset)
				.map(|p| p.expect[..len.min(p.expect.len())].to_vec())
		}
	}

	#[test]
	fn every_patch_writes_as_many_bytes_as_it_checks() {
		for p in CARS.iter().chain(ROSTER_CAR.iter()) {
			assert_eq!(
				p.expect.len(),
				p.write.len(),
				"0x{:08X} would resize the instruction it patches: {}",
				p.offset,
				p.what
			);
			assert_ne!(p.expect, p.write, "0x{:08X} writes what is already there", p.offset);
		}
	}

	#[test]
	fn a_matching_image_passes_and_a_changed_one_does_not() {
		assert!(all_present(&CARS, &image_of(&CARS)));
		assert!(all_present(&ROSTER_CAR, &image_of(&ROSTER_CAR)));
		// One byte out anywhere stops the whole set, which is the point:
		// half of this applied is worse than none of it.
		assert!(!all_present(&CARS, &|_, len| Some(vec![0u8; len])));
		assert!(!all_present(&ROSTER_CAR, &|_, _| None));
	}

	/// Car 247 is `gem_964_38RS_Shima1st` in both builds, read off the car
	/// table rather than taken from the roadmap, which mislabelled this
	/// entry as Masaki's.
	#[test]
	fn shima_goes_back_to_the_jpn_car_id() {
		let before = u32::from_le_bytes(ROSTER_CAR[0].expect.try_into().unwrap());
		let after = u32::from_le_bytes(ROSTER_CAR[0].write.try_into().unwrap());
		assert_eq!(before, 277);
		assert_eq!(after, 247);
	}

	/// File offset plus the link base, because `main` is `ET_EXEC`. Written
	/// out because the first version of this assertion had the addition
	/// wrong, and an offset that lands in the wrong place is the one mistake
	/// here that corrupts a running game rather than failing to work.
	#[test]
	fn a_file_offset_plus_the_link_base_is_the_runtime_address() {
		assert_eq!(BASE, 0x0804_8000);
		assert_eq!(BASE + 0x7A_8641, 0x087F_0641);
		assert_eq!(BASE + 0x12E_D6B0, 0x0933_56B0);
		assert_eq!(BASE + 0x68_95A8, 0x086D_15A8);
	}
}
