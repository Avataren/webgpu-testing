#ifndef LUA_WASM_ERRNO_H
#define LUA_WASM_ERRNO_H

extern int errno;

#define ENOENT 2
#define EBADF 9
#define ENOMEM 12
#define EACCES 13
#define EINVAL 22
#define ERANGE 34
#define ENOSYS 38

#endif
