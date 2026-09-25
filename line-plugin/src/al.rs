//! Redirects the game's OpenAL functions to OpenAL Soft (soft_oal.dll).
//! Ported from src/al.rs. OpenAL's Windows API is cdecl, like the game's
//! Linux calls, so a direct jump to the real function is safe.

use crate::line;
use std::ffi::{c_void, CString};

const FUNCS: [&str; 69] = [
	"alcSuspendContext",
	"alcCloseDevice",
	"alListener3f",
	"alGetFloat",
	"alcGetString",
	"alIsExtensionPresent",
	"alcIsExtensionPresent",
	"alGetBooleanv",
	"alDopplerFactor",
	"alSourcePausev",
	"alDisable",
	"alcGetCurrentContext",
	"alEnable",
	"alDeleteBuffers",
	"alGetSourcef",
	"alSourcefv",
	"alGetIntegerv",
	"alGetDouble",
	"alGetEnumValue",
	"alSourcei",
	"alSourceRewind",
	"alcDestroyContext",
	"alGetSourcefv",
	"alGetBufferi",
	"alSourcePlay",
	"alSourcef",
	"alSourceStop",
	"alcGetError",
	"alGetSource3f",
	"alSource3f",
	"alGetListenerfv",
	"alGetError",
	"alIsBuffer",
	"alcGetContextsDevice",
	"alGetListener3f",
	"alcGetIntegerv",
	"alDopplerVelocity",
	"alSourcePlayv",
	"alSourceUnqueueBuffers",
	"alGetFloatv",
	"alcOpenDevice",
	"alcProcessContext",
	"alListeneri",
	"alListenerfv",
	"alDistanceModel",
	"alSourcePause",
	"alGenSources",
	"alIsEnabled",
	"alcMakeContextCurrent",
	"alDeleteSources",
	"alcGetEnumValue",
	"alSourceStopv",
	"alGetProcAddress",
	"alSourceRewindv",
	"alGetString",
	"alBufferData",
	"alIsSource",
	"alGetInteger",
	"alGetSourcei",
	"alSourceQueueBuffers",
	"alListenerf",
	"alGenBuffers",
	"alcGetProcAddress",
	"alGetBufferf",
	"alGetListeneri",
	"alGetDoublev",
	"alcCreateContext",
	"alGetBoolean",
	"alGetListenerf",
];

pub unsafe fn init() {
	let openal = crate::load_library("soft_oal.dll");
	if openal.is_null() {
		log!("al: soft_oal.dll not loaded, OpenAL not redirected");
		return;
	}

	let mut patched = 0;
	for func in FUNCS {
		let func_c = CString::new(func).unwrap();
		let real_func = crate::get_proc_address(openal, &func_c);
		if real_func.is_null() {
			log!("al: {func} not found in soft_oal.dll");
			continue;
		}
		let func_ptr = line::dl_sym(func);
		if func_ptr.is_null() {
			log!("al: {func} not found in main");
			continue;
		}
		if func_ptr == real_func {
			continue;
		}
		line::patch_bytes(func_ptr as *mut u8, &jump_to(real_func));
		patched += 1;
	}
	log!("al: redirected {patched}/{} functions to soft_oal.dll", FUNCS.len());
}

// mov eax, target; jmp eax
fn jump_to(target: *mut c_void) -> [u8; 7] {
	let target = (target as u32).to_le_bytes();
	[0xB8, target[0], target[1], target[2], target[3], 0xFF, 0xE0]
}
