use crate::*;

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

pub unsafe fn load_al_funcs() {
	for func in FUNCS {
		load_al_func(func);
	}
}

unsafe fn load_al_func(func: &str) {
	let openal_module = platform::load_library(platform::OPENAL_LIBRARY);
	let real_func = platform::library_symbol(openal_module, func);
	if real_func.is_null() {
		panic!("{func} not found in {}", platform::OPENAL_LIBRARY);
	}

	let func_ptr = hook::get_symbol(func) as *mut c_void;
	if func_ptr.is_null() {
		panic!("{func} not found in main");
	}
	assert_ne!(func_ptr, real_func);

	// OpenAL is cdecl on both platforms, so a direct jump to it is safe.
	let real_func = real_func as usize;
	let real_func = real_func.to_le_bytes();
	let mut data = Vec::with_capacity(7);
	data.push(0xB8);
	for i in real_func {
		data.push(i);
	}
	data.push(0xFF);
	data.push(0xE0);

	hook::write_memory(func_ptr as *mut (), &data);
}
