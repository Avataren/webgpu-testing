#ifndef LUA_WASM_STDLIB_H
#define LUA_WASM_STDLIB_H
#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

void *malloc(size_t size);
void *calloc(size_t n, size_t size);
void *realloc(void *ptr, size_t size);
void free(void *ptr);
_Noreturn void abort(void);
_Noreturn void exit(int status);
char *getenv(const char *name);
int system(const char *cmd);
double strtod(const char *s, char **end);
int abs(int x);

#endif
