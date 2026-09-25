//! Logs fatal CPU exceptions (address, module, registers, top of stack) to
//! wal_line.log before Cygwin's handler takes over, so a crash anywhere in the
//! process shows where it happened.

use super::line;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

#[link(name = "kernel32")]
extern "system" {
	fn AddVectoredExceptionHandler(
		first: u32,
		handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32,
	) -> *mut c_void;
	fn GetModuleHandleExA(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
	fn GetModuleFileNameA(module: *mut c_void, name: *mut u8, size: u32) -> u32;
	fn VirtualQuery(
		address: *const c_void,
		info: *mut MemoryBasicInformation,
		size: usize,
	) -> usize;
}

#[repr(C)]
#[derive(Default)]
struct MemoryBasicInformation {
	base_address: usize,
	allocation_base: usize,
	allocation_protect: u32,
	region_size: usize,
	state: u32,
	protect: u32,
	kind: u32,
}

/// How many bytes from `address` onwards can be read without faulting.
unsafe fn readable_bytes(address: usize) -> usize {
	let mut info = MemoryBasicInformation::default();
	if VirtualQuery(
		address as *const c_void,
		&mut info,
		size_of::<MemoryBasicInformation>(),
	) == 0
	{
		return 0;
	}
	const MEM_COMMIT: u32 = 0x1000;
	const PAGE_GUARD: u32 = 0x100;
	const READABLE: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
	if info.state != MEM_COMMIT || info.protect & PAGE_GUARD != 0 || info.protect & READABLE == 0 {
		return 0;
	}
	info.base_address + info.region_size - address
}

#[repr(C)]
struct ExceptionRecord {
	code: u32,
	flags: u32,
	record: *mut ExceptionRecord,
	address: *mut c_void,
	parameter_count: u32,
	information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
	record: *mut ExceptionRecord,
	context: *mut u8,
}

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
const MAX_REPORTS: u32 = 8;

static REPORTS: AtomicU32 = AtomicU32::new(0);

pub fn install() {
	unsafe {
		if AddVectoredExceptionHandler(1, handler).is_null() {
			crate::wal_log!("crash handler not installed");
		}
	}
}

fn describe(code: u32) -> Option<&'static str> {
	Some(match code {
		0xC0000005 => "access violation",
		0xC000001D => "illegal instruction",
		0xC0000096 => "privileged instruction",
		0xC0000094 => "integer divide by zero",
		0xC00000FD => "stack overflow",
		0xC0000409 => "stack buffer overrun",
		0x80000002 => "datatype misalignment",
		_ => return None,
	})
}

/// Names the module containing `address`: a Windows DLL/EXE, or the game's main ELF.
unsafe fn locate(address: usize) -> String {
	// GetModuleHandleExA treats a null address as "the main executable".
	if address < 0x10000 {
		return "unknown (null or small value)".to_string();
	}
	let mut module = std::ptr::null_mut();
	if GetModuleHandleExA(
		GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
		address as *const c_void,
		&mut module,
	) != 0
	{
		let mut name = [0u8; 260];
		let len = GetModuleFileNameA(module, name.as_mut_ptr(), name.len() as u32) as usize;
		let path = String::from_utf8_lossy(&name[..len]).into_owned();
		let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
		return format!("{file}+{:#x}", address - module as usize);
	}
	if let Some((start, size)) = line::main_module_range() {
		if (start..start + size).contains(&address) {
			return format!("main (ELF) {address:#x}");
		}
	}
	"unknown module (another ELF library or heap)".to_string()
}

unsafe extern "system" fn handler(pointers: *mut ExceptionPointers) -> i32 {
	let record = &*(*pointers).record;
	let Some(what) = describe(record.code) else {
		return EXCEPTION_CONTINUE_SEARCH;
	};
	if REPORTS.fetch_add(1, Ordering::Relaxed) >= MAX_REPORTS {
		return EXCEPTION_CONTINUE_SEARCH;
	}

	let address = record.address as usize;
	let mut msg = format!(
		"CRASH (first chance): {what} ({:#010x}) at {address:#010x} in {}",
		record.code,
		locate(address)
	);
	if record.code == 0xC0000005 && record.parameter_count >= 2 {
		let access = match record.information[0] {
			0 => "reading",
			1 => "writing",
			8 => "executing",
			_ => "accessing",
		};
		msg += &format!(", {access} {:#010x}", record.information[1]);
	}

	// x86 CONTEXT field offsets.
	let ctx = (*pointers).context;
	let reg = |offset: usize| (ctx.add(offset) as *const u32).read_unaligned();
	let esp = reg(0xC4);
	msg += &format!(
		"\n  eax={:08x} ebx={:08x} ecx={:08x} edx={:08x} esi={:08x} edi={:08x} ebp={:08x} esp={:08x} eip={:08x}",
		reg(0xB0),
		reg(0xA4),
		reg(0xAC),
		reg(0xA8),
		reg(0xA0),
		reg(0x9C),
		reg(0xB4),
		esp,
		reg(0xB8)
	);

	// Values on the stack that point into known modules are likely return addresses.
	msg += "\n  stack:";
	let slots = (readable_bytes(esp as usize) / 4).min(32);
	for i in 0..slots {
		let value = ((esp as usize + i * 4) as *const u32).read_volatile() as usize;
		let place = locate(value);
		if !place.starts_with("unknown") {
			msg += &format!("\n    [esp+{:#04x}] {value:#010x} {place}", i * 4);
		}
	}

	super::log::write_nonblocking(&msg);
	EXCEPTION_CONTINUE_SEARCH
}
