/*
 * Stand-ins for game functions, compiled like the game's Linux code: they
 * assume a 16-byte aligned stack on entry (-mincoming-stack-boundary=4) and
 * use aligned SSE stores, so a call with a misaligned stack faults here just
 * as it would in the real game.
 */
#include <string.h>

typedef float v4 __attribute__((vector_size(16)));

#define ALIGNED_TOUCH()                                                   \
	do {                                                                  \
		v4 tmp;                                                           \
		__asm__ volatile("movaps %1, %0" : "=m"(tmp) : "x"((v4){1, 2, 3, 4})); \
	} while (0)

int cout_object;
int cl_main_ran;
void game_cl_main(void **self)
{
	ALIGNED_TOUCH();
	self[0] = (void *)0x1111;
	cl_main_ran = 1;
}

double pushed_number;
int set_global_calls;
int getglobal_calls;
void game_lua_pushnumber(void *state, double value)
{
	ALIGNED_TOUCH();
	(void)state;
	pushed_number = value;
}
void game_lua_setglobal(void *state, const char *name)
{
	ALIGNED_TOUCH();
	(void)state;
	(void)name;
	set_global_calls++;
}
int game_lua_getglobal(void *state, const char *name)
{
	ALIGNED_TOUCH();
	(void)state;
	(void)name;
	getglobal_calls++;
	return 77;
}

int viewport_w, viewport_h;
float viewport_a5;
void *game_set_viewport(void *self, int a1, int a2, int w, int h, float a5, float a6)
{
	ALIGNED_TOUCH();
	(void)a1;
	(void)a2;
	(void)a6;
	viewport_w = w;
	viewport_h = h;
	viewport_a5 = a5;
	return self;
}

float perspective_fov, perspective_aspect;
void game_make_perspective(void *self, float fov, float a2, float aspect, float a4, float a5)
{
	ALIGNED_TOUCH();
	(void)self;
	(void)a2;
	(void)a4;
	(void)a5;
	perspective_fov = fov;
	perspective_aspect = aspect;
}

/* Main-thread plumbing used by adm.rs. */
int is_main_thread = 1;
int app_instance_object;
void *game_app_get_instance(void)
{
	ALIGNED_TOUCH();
	return &app_instance_object;
}
_Bool game_app_is_main_thread(void *app)
{
	ALIGNED_TOUCH();
	return app == &app_instance_object && is_main_thread;
}
int thread_current_object;
void *game_thread_manager_current(void *manager)
{
	ALIGNED_TOUCH();
	(void)manager;
	return &thread_current_object;
}
int main_thread_calls;
void game_call_from_main_thread(void *thread, void (*f)(void *), void *args)
{
	ALIGNED_TOUCH();
	(void)thread;
	/* Emulate the main thread running the callback right away. */
	main_thread_calls++;
	is_main_thread = 1;
	f(args);
	is_main_thread = 0;
}
int texture_handle_calls, texture_handle_a1;
int game_create_texture_handle(void *self, int a1, int a2)
{
	ALIGNED_TOUCH();
	(void)self;
	(void)a2;
	texture_handle_calls++;
	texture_handle_a1 = a1;
	return 5;
}
