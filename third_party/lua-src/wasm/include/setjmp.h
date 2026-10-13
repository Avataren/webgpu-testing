#ifndef LUA_WASM_SETJMP_H
#define LUA_WASM_SETJMP_H
/* Lowered by clang (-mllvm -wasm-enable-sjlj) to wasm exception handling;
   the runtime is in ../sjlj.c and ../sjlj_tag.S. */

typedef unsigned long long jmp_buf[4];

int setjmp(jmp_buf env);
_Noreturn void longjmp(jmp_buf env, int val);

#endif
