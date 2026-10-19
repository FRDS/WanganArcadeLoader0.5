/*
 * Calls into the game's code from Rust with a 16-byte aligned stack.
 *
 * The game was built by Linux gcc, which assumes the stack is 16-byte aligned
 * at every call instruction. Rust on i686 Windows only keeps 4-byte alignment,
 * so every call from Rust into game code goes through here. `args` holds the
 * arguments as 32-bit stack words (a double takes two), copied below an
 * aligned stack pointer before the cdecl call.
 *
 * wal_call_int returns eax:edx (void, integers, pointers, bool in al).
 * wal_call_float returns st(0) (float and double results).
 */
#include <stdint.h>

#define WAL_CALL_BODY                                        \
	"mov %%esp, %%edi\n\t"                                   \
	"lea (,%%ecx,4), %%eax\n\t"                              \
	"sub %%eax, %%esp\n\t"                                   \
	"and $0xfffffff0, %%esp\n\t"                             \
	"mov %%esp, %%edx\n\t"                                   \
	"1:\n\t"                                                 \
	"test %%ecx, %%ecx\n\t"                                  \
	"jz 2f\n\t"                                              \
	"mov (%%esi), %%eax\n\t"                                 \
	"mov %%eax, (%%edx)\n\t"                                 \
	"add $4, %%esi\n\t"                                      \
	"add $4, %%edx\n\t"                                      \
	"dec %%ecx\n\t"                                          \
	"jmp 1b\n\t"                                             \
	"2:\n\t"                                                 \
	"call *%%ebx\n\t"                                        \
	"mov %%edi, %%esp\n\t"

uint64_t wal_call_int(void *f, uint32_t count, const uint32_t *args)
{
	uint32_t lo, hi;
	__asm__ volatile(WAL_CALL_BODY
					 : "=a"(lo), "=d"(hi), "+c"(count), "+S"(args)
					 : "b"(f)
					 : "edi", "memory", "cc");
	return ((uint64_t)hi << 32) | lo;
}

double wal_call_float(void *f, uint32_t count, const uint32_t *args)
{
	double result;
	__asm__ volatile(WAL_CALL_BODY
					 : "=t"(result), "+c"(count), "+S"(args)
					 : "b"(f)
					 : "eax", "edx", "edi", "memory", "cc");
	return result;
}


/*
 * Captures who asked for a car.
 *
 * `nsCarParts::LuaNickNameGetFromModelID` is detoured here. The detour leaves
 * the caller's return address where a normal call puts it, so
 * `__builtin_return_address(0)` is the game's call site. Rust cannot read a
 * return address portably; C can, and this file is already compiled for both
 * targets.
 *
 * One frame is all this gets. Scanning further up the stack for the rest of
 * the chain was tried and does not work -- see `carparts.rs`.
 */
extern const char *wal_nickname_seen(int model_id, void *caller);

const char *wal_nickname_trampoline(int model_id)
{
	return wal_nickname_seen(model_id, __builtin_return_address(0));
}

/*
 * The same trick one frame up.
 *
 * Every car -- the player's and every rival's -- reaches
 * `clCarModelData::constructBody(clCarConstructParam const&)` with its model
 * id already in the param, so that frame cannot tell a story rival from
 * anything else. Whoever *fills in* the param can.
 */
extern void *wal_construct_body_seen(void *self, const void *param, void *caller);

void *wal_construct_body_trampoline(void *self, const void *param)
{
	return wal_construct_body_seen(self, param, __builtin_return_address(0));
}
