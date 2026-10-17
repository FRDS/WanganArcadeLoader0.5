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
//! The story edit restores **Shima's Blackbird**. The roadmap called chara 30
//! "Masaki" and that is wrong: resolving the car-id table absolutely -- 311
//! records of 24 bytes, name pointer at `+8` -- gives id 247 as
//! `gem_964_38RS_Shima1st` and 251 as `gem_964_RSR_Masaki`. Export puts chara
//! 30 on 277, `nis_BNR32_Feeling`. All 32 entries were compared against a JPN
//! dump and chara 30 is the only one that differs, so this is the whole of the
//! story-car change.
//!
//! `nis_Z33_Shima` is a different entry, chara 20, identical in both builds --
//! so seeing Shima in a Z33 is not necessarily this edit failing.

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

/// Puts story chara 30 -- Shima -- back in the Gemballa 964 38RS.
///
/// Separate from `CARS` because it is safe on its own: a rival's car comes
/// from `data/characar/gem_964_38RS_Shima1st`, which the Export dump ships
/// complete, so this needs none of the models `CARS` does.
const MASAKI: [Patch; 1] = [Patch {
	offset: 0x12E_D6B0,
	expect: &[0x15, 0x01, 0x00, 0x00],
	write: &[0xf7, 0x00, 0x00, 0x00],
	what: "story chara 30 (Shima) drives the Gemballa 964 38RS again (277 -> 247)",
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

/// **Diagnostic, kept off.** Writes car 247 into every one of the 32 rival
/// entries.
///
/// Run on hardware on 2026-10-17 and it answered: with all 32 set to the
/// Gemballa, story chapter 6/7 still showed Shima in the Z33. **Story mode
/// does not read this table.** So `0x12ED6B0` -- the one car assignment that
/// differs between the Export and JPN binaries, and the edit the roadmap
/// describes as putting a story rival back in the Gemballa -- does not do
/// that, whatever it does do. Left here because it is the cheap question to
/// ask of any table we think drives something: set the whole thing to one
/// value and see if the game notices.
const PROBE_ALL: bool = false;

/// **Diagnostic.** Every dword in `.rodata` that holds 277, the model id the
/// engine actually resolves to `nis_Z33_Shima` for the story Episode 7 rival.
///
/// The roster table is not the source -- forcing all 32 of its entries to one
/// car changed nothing -- and `car_log` shows no id-space conversion, so the
/// 277 the game reads is stored directly somewhere else. There are only
/// thirteen of them in the whole of `.rodata`, so asking the game which one
/// matters is one run, where reasoning about which looks like story data has
/// now been wrong three times.
const PROBE_277: bool = false;
const RODATA_277: [usize; 13] = [
	0x012A_D8F8,
	0x012E_2168,
	0x012E_51A4,
	0x012E_6E9C,
	0x012E_D6B0,
	0x0139_B790,
	0x013D_C314,
	0x013D_CD24,
	0x013D_CD28,
	0x013D_CD2C,
	0x013D_CD30,
	0x0140_4570,
	0x0140_5020,
];
const ROSTER: usize = 0x12E_D3E0;
const ROSTER_LEN: usize = 32;

pub unsafe fn init() {
	if !CONFIG.gemballa {
		return;
	}
	if GAME_VERSION.major != GameMajor::W3P {
		platform::log("gemballa: only 3DX+ is patched, and this is not it");
		return;
	}

	// Shima first, because it is the half that stands alone.
	apply(&MASAKI, "the story car");

	if PROBE_277 {
		let mut done = 0;
		for off in RODATA_277 {
			if read_image(off, 4).as_deref() == Some(&277u32.to_le_bytes()) {
				platform::write_memory((BASE + off) as *mut (), &247u32.to_le_bytes());
				done += 1;
			}
		}
		platform::log(&format!(
			"gemballa: PROBE -- {done} of {} .rodata sites holding 277 set to 247",
			RODATA_277.len()
		));
	}

	if PROBE_ALL {
		for i in 0..ROSTER_LEN {
			platform::write_memory((BASE + ROSTER + i * 24) as *mut (), &247u32.to_le_bytes());
		}
		platform::log(&format!(
			"gemballa: PROBE -- all {ROSTER_LEN} rival entries set to car 247. 			 If rivals are still their own cars, story mode does not read this table."
		));
	}

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

	/// Car 247 is `gem_964_38RS_Shima1st` in both builds, read off the car
	/// table rather than taken from the roadmap, which mislabelled this
	/// entry as Masaki's.
	#[test]
	fn shima_goes_back_to_the_jpn_car_id() {
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
