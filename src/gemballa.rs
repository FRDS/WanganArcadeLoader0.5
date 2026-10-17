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
//! What the unlock does *not* do, measured against a JPN dump on 2026-10-17:
//! touch any story car but Masaki's. All 32 entries in the table were compared
//! and his is the only one that differs, so the rivals who already drive
//! Gemballas in this build -- Shima's 964 among them -- were never hidden and
//! need nothing done to them.

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

/// Puts story chara 30 back in the Gemballa 964 RSR.
///
/// Separate from `CARS` because it is safe on its own: a rival's car comes
/// from `data/characar/gem_964_RSR_Masaki`, which the Export dump ships
/// complete, so this needs none of the models `CARS` does.
const MASAKI: [Patch; 1] = [Patch {
	offset: 0x12E_D6B0,
	expect: &[0x15, 0x01, 0x00, 0x00],
	write: &[0xf7, 0x00, 0x00, 0x00],
	what: "story chara 30 drives the Gemballa 964 RSR again (277 -> 247)",
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
		platform::log(&format!("gemballa: patched 0x{:08X} -- {}", p.offset, p.what));
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

pub unsafe fn init() {
	if !CONFIG.gemballa {
		return;
	}
	if GAME_VERSION.major != GameMajor::W3P {
		platform::log("gemballa: only 3DX+ is patched, and this is not it");
		return;
	}

	// Masaki first, because it is the half that stands alone.
	apply(&MASAKI, "the story car");

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
		for p in CARS.iter().chain(MASAKI.iter()) {
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
		assert!(all_present(&MASAKI, &image_of(&MASAKI)));
		// One byte out anywhere stops the whole set, which is the point:
		// half of this applied is worse than none of it.
		assert!(!all_present(&CARS, &|_, len| Some(vec![0u8; len])));
		assert!(!all_present(&MASAKI, &|_, _| None));
	}

	/// The Masaki edit is the one documented car-id change, and 247 is what a
	/// JPN dump holds at that offset.
	#[test]
	fn masaki_goes_back_to_the_jpn_car_id() {
		let before = u32::from_le_bytes(MASAKI[0].expect.try_into().unwrap());
		let after = u32::from_le_bytes(MASAKI[0].write.try_into().unwrap());
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
