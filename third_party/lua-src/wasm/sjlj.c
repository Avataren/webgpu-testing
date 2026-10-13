/*
 * Runtime for clang's wasm setjmp/longjmp lowering (-mllvm -wasm-enable-sjlj),
 * after wasi-libc's libc-top-half/musl/src/setjmp/wasm32/rt.c. longjmp becomes
 * a throw of the __c_longjmp tag (sjlj_tag.S) that the setjmp caller catches.
 *
 * Must not call longjmp itself: the lowering renames longjmp to __wasm_longjmp.
 */

#include <stdint.h>

_Noreturn void lua_wasm_abort(const char *msg);

struct jmp_buf_impl {
  void *func_invocation_id;
  uint32_t label;
  struct arg {
    void *env;
    int val;
  } arg;
};

_Static_assert(sizeof(struct jmp_buf_impl) <= 32, "jmp_buf in setjmp.h is too small");

void __wasm_setjmp(void *env, uint32_t label, void *func_invocation_id) {
  struct jmp_buf_impl *jb = env;
  if (label == 0 || func_invocation_id == 0) lua_wasm_abort("bad __wasm_setjmp call");
  jb->func_invocation_id = func_invocation_id;
  jb->label = label;
}

uint32_t __wasm_setjmp_test(void *env, void *func_invocation_id) {
  struct jmp_buf_impl *jb = env;
  return jb->func_invocation_id == func_invocation_id ? jb->label : 0;
}

void __wasm_longjmp(void *env, int val) {
  struct jmp_buf_impl *jb = env;
  jb->arg.env = env;
  jb->arg.val = val == 0 ? 1 : val;
  __builtin_wasm_throw(1, &jb->arg); /* 1 = the C longjmp tag */
}
