/* Minimal stdio for building Lua on wasm32-unknown-unknown. See ../libc.c. */
#ifndef LUA_WASM_STDIO_H
#define LUA_WASM_STDIO_H
#include <stdarg.h>
#include <stddef.h>

typedef struct lua_wasm_file FILE;
typedef long long fpos_t;

extern FILE *const stdin;
extern FILE *const stdout;
extern FILE *const stderr;

#define EOF (-1)
#define BUFSIZ 1024
#define FILENAME_MAX 4096
#define L_tmpnam 32
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

FILE *fopen(const char *path, const char *mode);
FILE *freopen(const char *path, const char *mode, FILE *f);
FILE *tmpfile(void);
char *tmpnam(char *s);
int fclose(FILE *f);
int fflush(FILE *f);
int setvbuf(FILE *f, char *buf, int mode, size_t size);
size_t fread(void *ptr, size_t size, size_t n, FILE *f);
size_t fwrite(const void *ptr, size_t size, size_t n, FILE *f);
int fgetc(FILE *f);
int getc(FILE *f);
int ungetc(int c, FILE *f);
char *fgets(char *s, int n, FILE *f);
int fputs(const char *s, FILE *f);
int fseek(FILE *f, long off, int whence);
long ftell(FILE *f);
int feof(FILE *f);
int ferror(FILE *f);
void clearerr(FILE *f);
int remove(const char *path);
int rename(const char *from, const char *to);

int printf(const char *fmt, ...);
int fprintf(FILE *f, const char *fmt, ...);
int sprintf(char *buf, const char *fmt, ...);
int snprintf(char *buf, size_t n, const char *fmt, ...);
int vfprintf(FILE *f, const char *fmt, va_list ap);
int vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);

#endif
