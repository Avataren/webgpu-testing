#ifndef LUA_WASM_ASSERT_H
#define LUA_WASM_ASSERT_H

_Noreturn void lua_wasm_assert_fail(const char *expr, const char *file, int line);

#ifdef NDEBUG
#define assert(e) ((void)0)
#else
#define assert(e) ((e) ? (void)0 : lua_wasm_assert_fail(#e, __FILE__, __LINE__))
#endif

#endif
