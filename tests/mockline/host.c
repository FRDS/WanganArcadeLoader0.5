/*
 * A stand-in for LINE (https://github.com/axylol/line): loads wal_3dxp.dll,
 * hands it the same 8-entry function table and exported callbacks, then plays
 * the game's side of the boot sequence and checks the results.
 *
 * Every symbol the plugin asks for gets its own stub in read-write-execute
 * memory (as LINE maps the game), so hooks and code patches have real targets.
 * Symbols listed in FAKES resolve to specific stand-ins in game.c instead.
 */
#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <stdbool.h>

/* game.c */
extern int cout_object, cl_main_ran, set_global_calls, getglobal_calls;
extern double pushed_number;
extern int viewport_w, viewport_h;
extern float perspective_fov, perspective_aspect;
extern int is_main_thread, main_thread_calls, texture_handle_calls, texture_handle_a1;
void game_cl_main(void **);
void game_lua_pushnumber(void *, double);
void game_lua_setglobal(void *, const char *);
int game_lua_getglobal(void *, const char *);
void *game_set_viewport(void *, int, int, int, int, float, float);
void game_make_perspective(void *, float, float, float, float, float);
void *game_app_get_instance(void);
_Bool game_app_is_main_thread(void *);
void *game_thread_manager_current(void *);
void game_call_from_main_thread(void *, void (*)(void *), void *);
int game_create_texture_handle(void *, int, int);

static int failures;
#define CHECK(cond, what)                                                        \
	do {                                                                         \
		if (cond)                                                                \
			printf("HOST PASS: %s\n", what);                                     \
		else {                                                                   \
			printf("HOST FAIL: %s\n", what);                                     \
			failures++;                                                          \
		}                                                                        \
		fflush(stdout);                                                          \
	} while (0)

/* The game's RomInfo, as read by lib.rs. */
#pragma pack(push, 1)
static struct {
	char name[32], region[32], release_type[32], date[32], time[32];
	int revision;
	char revision_name[32];
} rom_info = {.revision_name = "W3P100-2-0-0-A01"};
#pragma pack(pop)

/* clMain singleton: [instance] -> object with the thread manager at +0x40. */
static char main_object[0x80];
static void *main_instance = main_object;

static struct {
	const char *name;
	void *target;
} FAKES[] = {
	{"_ZN6clMainC1Ev", game_cl_main},
	{"_ZSt4cout", &cout_object},
	{"gRomInfo", &rom_info},
	{"lua_pushnumber", game_lua_pushnumber},
	{"lua_setglobal", game_lua_setglobal},
	{"lua_getglobal", game_lua_getglobal},
	{"_ZN3Gap3Gfx19igAGLEVisualContext11setViewportEiiiiff", game_set_viewport},
	{"_ZN3Gap4Math11igMatrix44f32makePerspectiveProjectionRadiansEfffff", game_make_perspective},
	{"_ZN11clAppSystem11getInstanceEv", game_app_get_instance},
	{"_ZN11clAppSystem12isMainThreadEv", game_app_is_main_thread},
	{"_ZN17clNPThreadManager7currentEv", game_thread_manager_current},
	{"_ZN10clNPThread26callFunctionFromMainThreadEPFvPvES0_", game_call_from_main_thread},
	{"_ZN11teSingletonI10teSequenceI6clMainEE11sm_instanceE", &main_instance},
	{"_ZN24clAlchemyTextureAccessor19createTextureHandleEii", game_create_texture_handle},
};
#define NFAKES (sizeof(FAKES) / sizeof(FAKES[0]))

/* Symbols handed out so far: name, address the plugin sees, current implementation. */
#define MAX_SYMS 512
static struct {
	char name[128];
	void *target;
	void *impl;
} syms[MAX_SYMS];
static int nsyms;

#define MODULE_SIZE 0x40000
#define USB_OFFSET 0x100
#define STUBS_OFFSET 0x1000
static unsigned char *module_mem;

static void *new_stub(int index)
{
	/* mov eax, 0x5000+index ; ret  (plus padding room for a 7-byte patch) */
	unsigned char *p = module_mem + STUBS_OFFSET + index * 16;
	int value = 0x5000 + index;
	p[0] = 0xB8;
	memcpy(p + 1, &value, 4);
	p[5] = 0xC3;
	return p;
}

static int find_sym(const char *name)
{
	for (int i = 0; i < nsyms; i++)
		if (strcmp(syms[i].name, name) == 0)
			return i;
	return -1;
}

static void *sym(const char *name)
{
	int i = find_sym(name);
	if (i >= 0)
		return syms[i].target;
	if (nsyms == MAX_SYMS)
		return NULL;
	i = nsyms++;
	strncpy(syms[i].name, name, sizeof(syms[i].name) - 1);
	syms[i].target = NULL;
	for (size_t f = 0; f < NFAKES; f++)
		if (strcmp(FAKES[f].name, name) == 0)
			syms[i].target = FAKES[f].target;
	if (!syms[i].target)
		syms[i].target = new_stub(i);
	syms[i].impl = syms[i].target;
	return syms[i].target;
}

/* What the game would call for `name`: the hook's detour if hooked. */
static void *impl(const char *name)
{
	int i = find_sym(name);
	return i >= 0 ? syms[i].impl : NULL;
}

/* LINE's stubs for libc functions the plugin hooks. */
static bool system_passed_through;
static int stub_system(const char *cmd)
{
	(void)cmd;
	system_passed_through = true;
	return 0;
}
static void stub_exit(int code)
{
	printf("HOST: plugin called exit(%d)\n", code);
	fflush(stdout);
	ExitProcess(3);
}
static void *stub_stdout_value = NULL;
static struct {
	void *impl;
} system_stub = {stub_system};

/* The function table, in line/plugin.cpp order. */
static void *t_dlopen(const char *n) { (void)n; return NULL; }
static void *t_dlsym(void *h, const char *n) { (void)h; return sym(n); }
static int hooks;
static void t_hook(void *target, void *detour, void **original)
{
	if (target == (void *)stub_system) {
		system_stub.impl = detour;
		if (original) *original = target;
		hooks++;
		return;
	}
	for (int i = 0; i < nsyms; i++)
		if (syms[i].target == target) {
			syms[i].impl = detour;
			if (original) *original = target;
			hooks++;
			return;
		}
}
static void *t_module_by_name(const char *n) { (void)n; return NULL; }
static void *t_module_start(void *m) { return m ? module_mem : NULL; }
static unsigned t_module_size(void *m) { return m ? MODULE_SIZE : 0; }
static void *t_module_by_base(void *h) { return h == NULL ? (void *)1 : NULL; }
static void *t_resolve_stub(const char *n)
{
	if (strcmp(n, "system") == 0) return (void *)stub_system;
	if (strcmp(n, "exit") == 0) return (void *)stub_exit;
	if (strcmp(n, "stdout") == 0) return &stub_stdout_value;
	return NULL;
}
static void *table[8] = {
	t_dlopen, t_dlsym, t_hook, t_module_by_name,
	t_module_start, t_module_size, t_module_by_base, t_resolve_stub,
};

static bool file_equals(const char *path, const char *expected)
{
	char buf[512] = {0};
	FILE *f = fopen(path, "rb");
	if (!f) return false;
	size_t n = fread(buf, 1, sizeof buf - 1, f);
	fclose(f);
	return n == strlen(expected) && memcmp(buf, expected, n) == 0;
}

int main(int argc, char **argv)
{
	const char *plugin = argc > 1 ? argv[1] : "wal_3dxp.dll";
	module_mem = VirtualAlloc(NULL, MODULE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
	memset(module_mem, 0xCC, MODULE_SIZE);
	strcpy((char *)module_mem + USB_OFFSET, "/proc/bus/usb/devices");

	CreateDirectoryA("tmp", NULL);
	CreateDirectoryA("tmp\\data", NULL);
	CreateDirectoryA("tmp\\data\\target", NULL);
	fclose(fopen("tmp\\data\\target\\b.target.gz", "wb"));
	fclose(fopen("tmp\\data\\target\\a.target.gz", "wb"));

	HMODULE dll = LoadLibraryA(plugin);
	CHECK(dll != NULL, "plugin DLL loads");
	if (!dll) return 1;
	void (*OnInitialize)(int, void **) = (void *)GetProcAddress(dll, "OnInitialize");
	void (*OnPreExecute)(const char *, void *) = (void *)GetProcAddress(dll, "OnPreExecute");
	bool (*OnDlOpen)(const char *, void **) = (void *)GetProcAddress(dll, "OnDlOpen");
	bool (*OnDlSym)(void *, const char *, void **) = (void *)GetProcAddress(dll, "OnDlSym");
	CHECK(OnInitialize && OnPreExecute && OnDlOpen && OnDlSym, "all 4 exports found");

	OnInitialize(1, table);
	OnPreExecute("libstdc++.so.6", (void *)0x1000);
	CHECK(hooks == 0, "nothing hooked before main");
	OnPreExecute("main", NULL);
	printf("HOST: %d hooks, %d symbols requested\n", hooks, nsyms);
	CHECK(hooks >= 50, "boot hooks installed");

	void *res = (void *)0x55;
	CHECK(!OnDlOpen("libfoo.so", &res) && res == (void *)0x55, "OnDlOpen ignores other libraries");
	CHECK(!OnDlSym(NULL, "cgCreateContext", &res), "OnDlSym ignores the NULL handle");

	/* Dongle */
	int id = 0;
	CHECK(((int (*)(int, int, int *))impl("hasp_login"))(0, 0, &id) == 0 && id == 1, "hasp_login");
	unsigned char mem[0xD40];
	CHECK(((int (*)(int, int, int, int, unsigned char *))impl("hasp_read"))(0, 0, 0, 0xD40, mem) == 0 &&
			  memcmp(mem + 0xD00, "285013501138", 12) == 0,
		  "hasp_read returns the dongle id");
	CHECK(((int (*)(void))impl("_ZNK6clHasp7isAvailEv"))() == 1, "clHasp::isAvail -> 1");
	CHECK(((int (*)(void))impl("_ZN18clSeqBootNetThread3runEPv"))() == 1, "network boot skipped");
	CHECK(((int (*)(void))impl("_ZN5clNet19setInterfaceAddressEv"))() == 1, "setInterfaceAddress stubbed");

	/* Windows platform setup */
	CHECK(strcmp((char *)module_mem + USB_OFFSET, "tmp/usb-devices") == 0, "usb-devices path patched");
	int (*sys)(const char *) = system_stub.impl;
	sys("find /tmp/data/target/ -type f -name \"*.target.gz\"  -type f | sort >/tmp/find.txt");
	CHECK(file_equals("tmp\\find.txt", "tmp/data/target/a.target.gz\ntmp/data/target/b.target.gz\n"),
		  "system(find) writes a sorted tmp/find.txt");
	sys("rm -rf /");
	CHECK(!system_passed_through, "other system() commands are not run");

	/* OpenAL: the game's alGenSources now jumps into soft_oal.dll. */
	CHECK(((int (*)(void))sym("alGenSources"))() == 0xA1, "alGenSources redirected to soft_oal.dll");

	/* Rust -> game calls, into code that faults on a misaligned stack. */
	void *obj[4] = {0};
	((void (*)(void **))impl("_ZN6clMainC1Ev"))(obj);
	CHECK(cl_main_ran && obj[0] == &cout_object, "cl_main calls the original, then sets std::cout");

	int got = ((int (*)(void *, const char *))impl("lua_getglobal"))(NULL, "SCREEN_XSIZE");
	CHECK(pushed_number == 1280.0 && set_global_calls == 1, "SCREEN_XSIZE pushed as 1280.0 (double arg)");
	CHECK(got == 77 && getglobal_calls == 1, "lua_getglobal original called and its result returned");

	((void *(*)(void *, int, int, int, int, float, float))impl(
		"_ZN3Gap3Gfx19igAGLEVisualContext11setViewportEiiiiff"))(NULL, 0, 0, 88, 82, 1.5f, 0.0f);
	CHECK(viewport_w == 176 && viewport_h == 122, "minimap viewport scaled to 1280x720");

	((void (*)(void *, float, float, float, float, float))impl(
		"_ZN3Gap4Math11igMatrix44f32makePerspectiveProjectionRadiansEfffff"))(
		NULL, 1.0f, 0.1f, 640.0f / 480.0f, 1.0f, 1000.0f);
	CHECK(perspective_aspect > 1.77f && perspective_aspect < 1.78f, "perspective aspect set to 16:9 (float args)");

	/* Main-thread marshalling in adm.rs */
	int (*create_texture)(void *, int, int) = impl("_ZN24clAlchemyTextureAccessor19createTextureHandleEii");
	CHECK(create_texture(NULL, 3, 4) == 5 && texture_handle_calls == 1, "createTextureHandle on the main thread");
	is_main_thread = 0;
	create_texture(NULL, 9, 4);
	CHECK(main_thread_calls == 1 && texture_handle_calls == 2 && texture_handle_a1 == 9,
		  "createTextureHandle from another thread runs via the main thread");

	/* ADM */
	unsigned int **mode = ((unsigned int **(*)(void))impl("admChooseModeConfigi"))();
	CHECK(memcmp(mode[0], "MOCF", 4) == 0 && mode[0][6] == 1280 && mode[0][7] == 720,
		  "admChooseModeConfigi reports the configured 1280x720");

	printf("HOST RESULT: %s (%d failures)\n", failures ? "FAILED" : "ALL CHECKS PASSED", failures);
	fflush(stdout);
	ExitProcess(failures ? 2 : 0);
}
