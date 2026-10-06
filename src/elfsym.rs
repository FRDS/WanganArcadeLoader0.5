//! The game's own symbol table, so a logged address can say which function it
//! is in rather than only which module.
//!
//! Ported from tattoohanz's C++ LINE plugin (`elfsym.cpp`, GPL-3). The fact
//! that makes it this simple is in its header: **`main` is `ET_EXEC` and not
//! position-independent, so a link address is a runtime address** and nothing
//! needs relocating -- on WMMT3, 3DX and 3DX+ alike, with nothing hardcoded
//! per build.
//!
//! Only `.dynsym` is read, which is all the Export 3DX+ dump has -- it ships
//! with no `.symtab` at all. Measured against that `main`: 61,074 `.dynsym`
//! entries, of which **47,937 are functions with an address**, the same count
//! `readelf --dyn-syms` reports. So a static function is not in here and an
//! address inside one is reported as the module and offset it already was.
//!
//! One departure from the C++: it keeps no `st_size`, so its `resolve` names
//! the nearest preceding symbol however far away it is, and a heap address a
//! megabyte past the last function still comes back as that function plus an
//! enormous offset. We keep the size and answer only inside a function's own
//! body, leaving everything else to be reported as the module and address it
//! already was.

use std::io::{Read, Seek, SeekFrom};
use std::sync::OnceLock;

const EHDR_SIZE: usize = 52;
const SHDR_SIZE: usize = 40;
const SYM_SIZE: usize = 16;
const ELFCLASS32: u8 = 1;
const SHT_DYNSYM: u32 = 11;
const STT_FUNC: u8 = 2;

/// A section or string table bigger than this is a sign we are reading
/// something that is not the game, and not a reason to allocate for it.
const SANE_SECTION: u32 = 64 << 20;

/// Where the loader looks, in order. `start.bat` runs from the game folder, so
/// the first one is the usual answer.
const CANDIDATES: [&str; 3] = ["main", "./main", "../main"];

struct Function {
	address: u32,
	size: u32,
	/// Offset into `names`.
	name: u32,
}

pub struct Symbols {
	/// The section `.dynsym` points at with `sh_link`, verbatim.
	names: Vec<u8>,
	/// Every `STT_FUNC` with an address, sorted by address.
	functions: Vec<Function>,
}

static SYMBOLS: OnceLock<Symbols> = OnceLock::new();

fn u16_at(buffer: &[u8], offset: usize) -> u16 {
	u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
}

fn u32_at(buffer: &[u8], offset: usize) -> u32 {
	u32::from_le_bytes([
		buffer[offset],
		buffer[offset + 1],
		buffer[offset + 2],
		buffer[offset + 3],
	])
}

fn read_at(file: &mut (impl Read + Seek), offset: u64, length: usize) -> Option<Vec<u8>> {
	file.seek(SeekFrom::Start(offset)).ok()?;
	let mut buffer = vec![0_u8; length];
	file.read_exact(&mut buffer).ok()?;
	Some(buffer)
}

/// Reads an ELF32's `.dynsym`. `None` for anything that is not one, or has no
/// function symbols -- a stripped binary is not an error worth a message of
/// its own.
pub fn parse(file: &mut (impl Read + Seek)) -> Option<Symbols> {
	let header = read_at(file, 0, EHDR_SIZE)?;
	if header[..4] != *b"\x7fELF" || header[4] != ELFCLASS32 {
		return None;
	}
	let section_offset = u32_at(&header, 0x20) as u64;
	let section_count = u16_at(&header, 0x30) as usize;
	if u16_at(&header, 0x2E) as usize != SHDR_SIZE || section_count == 0 {
		return None;
	}

	let sections = read_at(file, section_offset, section_count * SHDR_SIZE)?;
	let section = |index: usize| &sections[index * SHDR_SIZE..(index + 1) * SHDR_SIZE];
	let symbol_section = (0..section_count).find(|index| {
		let section = section(*index);
		u32_at(section, 4) == SHT_DYNSYM && u32_at(section, 36) as usize == SYM_SIZE
	})?;

	let symbols = section(symbol_section);
	let string_section = u32_at(symbols, 24) as usize;
	if string_section >= section_count {
		return None;
	}
	let strings = section(string_section);
	let (symbols_at, symbols_size) = (u32_at(symbols, 16) as u64, u32_at(symbols, 20));
	let (strings_at, strings_size) = (u32_at(strings, 16) as u64, u32_at(strings, 20));
	if symbols_size > SANE_SECTION || strings_size > SANE_SECTION {
		return None;
	}

	let names = read_at(file, strings_at, strings_size as usize)?;
	let table = read_at(file, symbols_at, symbols_size as usize)?;
	let mut functions: Vec<Function> = (0..table.len() / SYM_SIZE)
		.map(|index| &table[index * SYM_SIZE..])
		.filter(|symbol| symbol[12] & 0xF == STT_FUNC)
		.map(|symbol| Function {
			address: u32_at(symbol, 4),
			size: u32_at(symbol, 8),
			name: u32_at(symbol, 0),
		})
		.filter(|function| function.address != 0 && (function.name as usize) < names.len())
		.collect();
	if functions.is_empty() {
		return None;
	}
	functions.sort_unstable_by_key(|function| function.address);
	Some(Symbols { names, functions })
}

impl Symbols {
	pub fn count(&self) -> usize {
		self.functions.len()
	}

	fn name(&self, function: &Function) -> Option<&str> {
		let from = &self.names[function.name as usize..];
		let end = from.iter().position(|byte| *byte == 0)?;
		std::str::from_utf8(&from[..end])
			.ok()
			.filter(|name| !name.is_empty())
	}

	/// The function containing `address`, and how far into it that is.
	///
	/// A symbol with no size answers only for its own first byte: it is
	/// usually hand-written assembly, and there is no way to tell where it
	/// ends, so guessing would be the unbounded attribution this avoids.
	pub fn resolve(&self, address: u32) -> Option<(&str, u32)> {
		let index = match self
			.functions
			.binary_search_by_key(&address, |function| function.address)
		{
			Ok(index) => index,
			// The first function starts after this address.
			Err(0) => return None,
			Err(index) => index - 1,
		};
		let function = &self.functions[index];
		let offset = address - function.address;
		if offset >= function.size.max(1) {
			return None;
		}
		Some((self.name(function)?, offset))
	}

	/// `symbol+0x1c`, or just the symbol at its first instruction.
	pub fn describe(&self, address: u32) -> Option<String> {
		let (name, offset) = self.resolve(address)?;
		if offset == 0 {
			Some(String::from(name))
		} else {
			Some(format!("{name}+{offset:#x}"))
		}
	}
}

/// Reads the game's symbols once, for the crash reporter. Says what it found,
/// because a report that cannot name functions should say why beforehand
/// rather than look like there were none to name.
pub fn load() {
	if SYMBOLS.get().is_some() {
		return;
	}
	for candidate in CANDIDATES {
		let Ok(mut file) = std::fs::File::open(candidate) else {
			continue;
		};
		match parse(&mut file) {
			Some(symbols) => {
				crate::platform::log(&format!(
					"elfsym: {} functions from {candidate}",
					symbols.count()
				));
				_ = SYMBOLS.set(symbols);
				return;
			}
			None => crate::platform::log(&format!(
				"elfsym: {candidate} has no usable .dynsym, so a crash cannot name functions"
			)),
		}
	}
	crate::platform::log("elfsym: no `main` beside us, so a crash cannot name functions");
}

/// `symbol+0x1c` for an address in the game, if we know one.
pub fn describe(address: u32) -> Option<String> {
	SYMBOLS.get()?.describe(address)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;

	/// Builds an ELF32 with one `.dynsym` and its string table, so the parser
	/// is tested against bytes rather than against the dump.
	struct Builder {
		names: Vec<u8>,
		symbols: Vec<u8>,
	}

	impl Builder {
		fn new() -> Self {
			// Index 0 of a string table is the empty name.
			Builder {
				names: vec![0],
				symbols: vec![0; SYM_SIZE],
			}
		}

		fn add(mut self, name: &str, address: u32, size: u32, kind: u8) -> Self {
			let at = self.names.len() as u32;
			self.names.extend_from_slice(name.as_bytes());
			self.names.push(0);
			self.symbols.extend_from_slice(&at.to_le_bytes());
			self.symbols.extend_from_slice(&address.to_le_bytes());
			self.symbols.extend_from_slice(&size.to_le_bytes());
			self.symbols.push(kind);
			self.symbols.extend_from_slice(&[0, 0, 1]); // other, shndx
			self
		}

		/// `shdr_size` and `sym_size` are arguments so the malformed cases can
		/// lie about them.
		fn build_with(&self, class: u8, shdr_size: u16, sym_size: u32) -> Vec<u8> {
			let mut elf = vec![0_u8; EHDR_SIZE];
			elf[..4].copy_from_slice(b"\x7fELF");
			elf[4] = class;

			let names_at = EHDR_SIZE;
			let symbols_at = names_at + self.names.len();
			let sections_at = symbols_at + self.symbols.len();
			elf[0x20..0x24].copy_from_slice(&(sections_at as u32).to_le_bytes());
			elf[0x2E..0x30].copy_from_slice(&shdr_size.to_le_bytes());
			elf[0x30..0x32].copy_from_slice(&3_u16.to_le_bytes());
			elf.extend_from_slice(&self.names);
			elf.extend_from_slice(&self.symbols);

			let mut section = |kind: u32, offset: usize, size: usize, link: u32, entsize: u32| {
				let mut header = vec![0_u8; SHDR_SIZE];
				header[4..8].copy_from_slice(&kind.to_le_bytes());
				header[16..20].copy_from_slice(&(offset as u32).to_le_bytes());
				header[20..24].copy_from_slice(&(size as u32).to_le_bytes());
				header[24..28].copy_from_slice(&link.to_le_bytes());
				header[36..40].copy_from_slice(&entsize.to_le_bytes());
				elf.extend_from_slice(&header);
			};
			section(0, 0, 0, 0, 0); // the null section, as every ELF has
			section(3, names_at, self.names.len(), 0, 0); // .dynstr
			section(SHT_DYNSYM, symbols_at, self.symbols.len(), 1, sym_size);
			elf
		}

		fn build(&self) -> Vec<u8> {
			self.build_with(ELFCLASS32, SHDR_SIZE as u16, SYM_SIZE as u32)
		}
	}

	fn game() -> Symbols {
		let elf = Builder::new()
			// Deliberately out of address order, and with one of each thing
			// the filter is supposed to drop.
			.add("_Z7seqMainb", 0x08500000, 0x40, STT_FUNC)
			.add("clMain::ctor", 0x08400000, 0x20, STT_FUNC)
			.add("gRomInfo", 0x09000000, 0xC4, 1) // STT_OBJECT, not a function
			.add("undefined", 0, 0x10, STT_FUNC) // no address
			.add("asm_thunk", 0x08600000, 0, STT_FUNC) // no size
			.build();
		parse(&mut Cursor::new(elf)).expect("the builder makes a parseable ELF")
	}

	#[test]
	fn keeps_only_functions_with_an_address_and_sorts_them() {
		let symbols = game();
		assert_eq!(symbols.count(), 3);
		assert_eq!(symbols.resolve(0x08400000).unwrap().0, "clMain::ctor");
		assert_eq!(symbols.resolve(0x08500000).unwrap().0, "_Z7seqMainb");
		assert_eq!(symbols.resolve(0x09000000), None, "named a data symbol");
	}

	#[test]
	fn answers_inside_a_function_and_nowhere_else() {
		let symbols = game();
		assert_eq!(
			symbols.describe(0x08400000).as_deref(),
			Some("clMain::ctor")
		);
		assert_eq!(
			symbols.describe(0x0840001C).as_deref(),
			Some("clMain::ctor+0x1c")
		);
		// One past its last byte belongs to nothing.
		assert_eq!(symbols.describe(0x08400020), None);
		// Nor does the gap between two functions, which is what the C++ would
		// have reported as clMain::ctor+0xfffe0.
		assert_eq!(symbols.describe(0x084F0000), None);
		// Nor anything below the first function.
		assert_eq!(symbols.describe(0x08000000), None);
	}

	#[test]
	fn a_symbol_with_no_size_answers_for_its_first_byte_only() {
		let symbols = game();
		assert_eq!(symbols.describe(0x08600000).as_deref(), Some("asm_thunk"));
		assert_eq!(symbols.describe(0x08600004), None);
	}

	#[test]
	fn refuses_what_is_not_an_elf32_with_symbols() {
		let builder = Builder::new().add("f", 0x1000, 0x10, STT_FUNC);
		assert!(parse(&mut Cursor::new(builder.build())).is_some());

		// 64-bit, a section header size we do not know, and a symbol entry
		// size that is not ours: each one means this is not the layout we are
		// about to index into.
		assert!(parse(&mut Cursor::new(builder.build_with(
			2,
			SHDR_SIZE as u16,
			16
		)))
		.is_none());
		assert!(parse(&mut Cursor::new(builder.build_with(ELFCLASS32, 64, 16))).is_none());
		assert!(parse(&mut Cursor::new(builder.build_with(
			ELFCLASS32,
			SHDR_SIZE as u16,
			24
		)))
		.is_none());

		// Not an ELF, empty, and truncated mid-table.
		assert!(parse(&mut Cursor::new(b"MZ\x90\x00 a PE, as it happens".to_vec())).is_none());
		assert!(parse(&mut Cursor::new(Vec::new())).is_none());
		let mut truncated = builder.build();
		truncated.truncate(truncated.len() - SHDR_SIZE - 4);
		assert!(parse(&mut Cursor::new(truncated)).is_none());

		// And a stripped binary: no functions is not an answer we can use.
		assert!(parse(&mut Cursor::new(Builder::new().build())).is_none());
	}
}
