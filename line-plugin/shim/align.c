/*
 * Calls from Rust back into the game's code go through here.
 *
 * The game was built by Linux gcc, which assumes the stack is 16-byte aligned
 * at every call instruction. Rust on i686 Windows only keeps 4-byte alignment,
 * so game code using SSE (movaps) on stack variables could fault if called
 * directly. This shim aligns the stack explicitly before the call.
 *
 * With WAL_MISALIGN defined, it instead leaves esp % 16 == 8 at the call, to
 * test whether the callee actually depends on alignment.
 */

typedef void (*wal_fn1)(void *);

void wal_call1(wal_fn1 f, void *a)
{
	__asm__ volatile(
		"mov %%esp, %%esi\n\t"
		"and $0xfffffff0, %%esp\n\t"
#ifdef WAL_MISALIGN
		"sub $4, %%esp\n\t"
#else
		"sub $12, %%esp\n\t"
#endif
		"push %1\n\t"
		"call *%0\n\t"
		"mov %%esi, %%esp\n\t"
		:
		: "b"(f), "D"(a)
		: "eax", "ecx", "edx", "esi", "memory", "cc");
}

/* Returns 1 for the aligned build, 0 for the misaligned diagnostic build. */
int wal_align_shim_is_aligned(void)
{
#ifdef WAL_MISALIGN
	return 0;
#else
	return 1;
#endif
}
