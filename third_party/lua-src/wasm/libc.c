/*
 * The slice of the C library that Lua 5.4 uses, for wasm32-unknown-unknown.
 *
 * That target has no libc, so this file supplies one. A few pieces need exact
 * algorithms or the host, and are implemented by the Rust program Lua is linked
 * into (see the `lua_wasm_*` declarations below and the engine's
 * `src/scripting/lua/wasm_libc.rs`):
 *
 *   - malloc/calloc/realloc/free
 *   - shortest-exact float formatting and decimal float parsing
 *   - console output, wall-clock time and aborting
 *
 * memcpy, memmove, memset, memcmp and the math functions (except frexp) come
 * from Rust's compiler-builtins. There is no filesystem: fopen and friends fail with ENOENT,
 * and stdout/stderr go to the browser console.
 */

#include <ctype.h>
#include <errno.h>
#include <locale.h>
#include <math.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* --- Implemented in Rust ------------------------------------------------- */

/* Formats a finite, non-negative value like printf "%.*e" (conv 'e') or "%.*f"
   (conv 'f'), but with Rust's exponent syntax ("1.5e-7"). Returns the length,
   truncating the output to cap - 1 bytes plus a terminating NUL. */
size_t lua_wasm_fmt_f64(char *out, size_t cap, double v, int prec, int conv);
/* Parses a decimal float that strtod already validated. */
double lua_wasm_parse_f64(const char *s, size_t len);
/* Writes to the console (fd 1 = stdout, 2 = stderr), line-buffered. */
void lua_wasm_write(int fd, const char *s, size_t len);
void lua_wasm_flush(int fd);
/* Milliseconds since the Unix epoch, and since an arbitrary start. */
double lua_wasm_time_ms(void);
double lua_wasm_clock_ms(void);
_Noreturn void lua_wasm_abort(const char *msg);

/* --- errno, process ------------------------------------------------------ */

int errno;

_Noreturn void abort(void) { lua_wasm_abort("abort() called"); }

_Noreturn void exit(int status) {
  char msg[48];
  snprintf(msg, sizeof msg, "exit(%d) called", status);
  lua_wasm_abort(msg);
}

_Noreturn void lua_wasm_assert_fail(const char *expr, const char *file, int line) {
  char msg[256];
  snprintf(msg, sizeof msg, "assertion failed: %s (%s:%d)", expr, file, line);
  lua_wasm_abort(msg);
}

char *getenv(const char *name) { (void)name; return NULL; }

/* system(NULL) == 0 tells Lua there is no shell. */
int system(const char *cmd) { return cmd == NULL ? 0 : -1; }

int abs(int x) { return x < 0 ? -x : x; }

char *strerror(int err) {
  switch (err) {
    case 0: return "Success";
    case ENOENT: return "No such file or directory";
    case EBADF: return "Bad file descriptor";
    case ENOMEM: return "Out of memory";
    case EACCES: return "Permission denied";
    case EINVAL: return "Invalid argument";
    case ERANGE: return "Result not representable";
    case ENOSYS: return "Function not implemented";
    default: return "Unknown error";
  }
}

/* --- math.h: the one function compiler-builtins lacks (musl's) ----------- */

double frexp(double x, int *e) {
  union { double d; uint64_t u; } y = {x};
  int ee = (int)(y.u >> 52 & 0x7ff);
  if (ee == 0) {
    if (x != 0.0) {
      x = frexp(x * 0x1p64, e);
      *e -= 64;
    } else {
      *e = 0;
    }
    return x;
  }
  if (ee == 0x7ff) return x;
  *e = ee - 0x3fe;
  y.u &= 0x800fffffffffffffULL;
  y.u |= 0x3fe0000000000000ULL;
  return y.d;
}

/* --- string.h ------------------------------------------------------------ */

void *memchr(const void *s, int c, size_t n) {
  const unsigned char *p = s;
  for (; n; n--, p++)
    if (*p == (unsigned char)c) return (void *)p;
  return NULL;
}

size_t strlen(const char *s) {
  const char *p = s;
  while (*p) p++;
  return (size_t)(p - s);
}

int strcmp(const char *a, const char *b) {
  while (*a && *a == *b) a++, b++;
  return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t n) {
  for (; n; n--, a++, b++) {
    if (*a != *b) return (unsigned char)*a - (unsigned char)*b;
    if (!*a) return 0;
  }
  return 0;
}

/* Only the "C" locale exists. */
int strcoll(const char *a, const char *b) { return strcmp(a, b); }

char *strcpy(char *dst, const char *src) {
  char *d = dst;
  while ((*d++ = *src++)) {}
  return dst;
}

char *strchr(const char *s, int c) {
  for (;; s++) {
    if (*s == (char)c) return (char *)s;
    if (!*s) return NULL;
  }
}

char *strrchr(const char *s, int c) {
  const char *found = NULL;
  for (;; s++) {
    if (*s == (char)c) found = s;
    if (!*s) return (char *)found;
  }
}

char *strstr(const char *s, const char *needle) {
  size_t n = strlen(needle);
  if (n == 0) return (char *)s;
  for (; *s; s++)
    if (*s == *needle && strncmp(s, needle, n) == 0) return (char *)s;
  return NULL;
}

size_t strspn(const char *s, const char *accept) {
  size_t n = 0;
  while (s[n] && strchr(accept, s[n])) n++;
  return n;
}

char *strpbrk(const char *s, const char *accept) {
  for (; *s; s++)
    if (strchr(accept, *s)) return (char *)s;
  return NULL;
}

/* --- ctype.h (ASCII, "C" locale) ----------------------------------------- */

int isdigit(int c) { return c >= '0' && c <= '9'; }
int islower(int c) { return c >= 'a' && c <= 'z'; }
int isupper(int c) { return c >= 'A' && c <= 'Z'; }
int isalpha(int c) { return islower(c) || isupper(c); }
int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isxdigit(int c) { return isdigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'); }
int isspace(int c) { return c == ' ' || (c >= '\t' && c <= '\r'); }
int iscntrl(int c) { return (c >= 0 && c < 32) || c == 127; }
int isprint(int c) { return c >= 32 && c < 127; }
int isgraph(int c) { return c > 32 && c < 127; }
int ispunct(int c) { return isgraph(c) && !isalnum(c); }
int tolower(int c) { return isupper(c) ? c + 32 : c; }
int toupper(int c) { return islower(c) ? c - 32 : c; }

/* --- locale.h ------------------------------------------------------------ */

static char decimal_point[] = ".";
static struct lconv c_lconv = {decimal_point};

struct lconv *localeconv(void) { return &c_lconv; }

char *setlocale(int category, const char *locale) {
  (void)category;
  if (locale == NULL || locale[0] == '\0' || strcmp(locale, "C") == 0 ||
      strcmp(locale, "POSIX") == 0)
    return "C";
  return NULL;
}

/* --- strtod -------------------------------------------------------------- */

static int lower(int c) { return c | 0x20; }

static int starts_with_ci(const char *s, const char *word) {
  for (; *word; s++, word++)
    if (lower((unsigned char)*s) != *word) return 0;
  return 1;
}

static int hex_value(int c) {
  if (isdigit(c)) return c - '0';
  return lower(c) - 'a' + 10;
}

static double parse_hex(const char *p, const char **end) {
  uint64_t mant = 0;
  long exp = 0;
  int any = 0, digits = 0;
  for (; isxdigit((unsigned char)*p); p++, any = 1) {
    if (digits < 15) {
      mant = mant * 16 + hex_value((unsigned char)*p);
      if (mant) digits++;
    } else {
      exp += 4;
    }
  }
  if (*p == '.') {
    for (p++; isxdigit((unsigned char)*p); p++, any = 1) {
      if (digits < 15) {
        mant = mant * 16 + hex_value((unsigned char)*p);
        if (mant) digits++;
        exp -= 4;
      }
    }
  }
  if (!any) return -1.0; /* no digits: caller parses just the "0" */
  if (lower((unsigned char)*p) == 'p') {
    const char *q = p + 1;
    int neg = 0;
    long e = 0;
    if (*q == '+' || *q == '-') neg = *q++ == '-';
    if (isdigit((unsigned char)*q)) {
      for (; isdigit((unsigned char)*q); q++)
        if (e < 100000) e = e * 10 + (*q - '0');
      exp += neg ? -e : e;
      p = q;
    }
  }
  *end = p;
  if (exp > 100000) exp = 100000;
  if (exp < -100000) exp = -100000;
  return ldexp((double)mant, (int)exp);
}

double strtod(const char *s, char **end) {
  const char *p = s, *start;
  int neg = 0;
  double v;
  while (isspace((unsigned char)*p)) p++;
  start = p;
  if (*p == '+' || *p == '-') neg = *p++ == '-';

  if (p[0] == '0' && lower((unsigned char)p[1]) == 'x') {
    const char *e;
    v = parse_hex(p + 2, &e);
    if (v < 0) { /* "0x" without digits is the number 0 */
      v = 0.0;
      e = p + 1;
    }
    if (end) *end = (char *)e;
    return neg ? -v : v;
  }
  if (starts_with_ci(p, "inf")) {
    p += starts_with_ci(p, "infinity") ? 8 : 3;
    if (end) *end = (char *)p;
    return neg ? -HUGE_VAL : HUGE_VAL;
  }
  if (starts_with_ci(p, "nan")) {
    if (end) *end = (char *)(p + 3);
    return neg ? -NAN : NAN;
  }

  int digits = 0;
  while (isdigit((unsigned char)*p)) p++, digits++;
  if (*p == '.') {
    p++;
    while (isdigit((unsigned char)*p)) p++, digits++;
  }
  if (!digits) {
    if (end) *end = (char *)s;
    return 0.0;
  }
  if (lower((unsigned char)*p) == 'e') {
    const char *q = p + 1;
    if (*q == '+' || *q == '-') q++;
    if (isdigit((unsigned char)*q)) {
      while (isdigit((unsigned char)*q)) q++;
      p = q;
    }
  }
  if (end) *end = (char *)p;
  v = lua_wasm_parse_f64(start, (size_t)(p - start));
  if (v == HUGE_VAL || v == -HUGE_VAL) errno = ERANGE;
  return v;
}

/* --- printf -------------------------------------------------------------- */

struct sink {
  char *buf; /* snprintf target, or NULL when writing to a stream */
  size_t cap;
  size_t len; /* bytes produced so far (may exceed cap) */
  FILE *file;
};

static void sink_put(struct sink *s, const char *p, size_t n) {
  if (s->file) {
    fwrite(p, 1, n, s->file);
  } else if (s->len < s->cap) {
    size_t room = s->cap - s->len;
    memcpy(s->buf + s->len, p, n < room ? n : room);
  }
  s->len += n;
}

static void sink_fill(struct sink *s, char c, int n) {
  char chunk[32];
  memset(chunk, c, sizeof chunk);
  while (n > 0) {
    int k = n < (int)sizeof chunk ? n : (int)sizeof chunk;
    sink_put(s, chunk, (size_t)k);
    n -= k;
  }
}

enum { F_LEFT = 1, F_PLUS = 2, F_SPACE = 4, F_ALT = 8, F_ZERO = 16 };

/* Writes prefix, `zeros` zeros, then body, padded to `width`. */
static void emit(struct sink *s, int flags, int width, const char *prefix, int zeros,
                 const char *body, size_t blen) {
  int plen = (int)strlen(prefix);
  int total = plen + zeros + (int)blen;
  int pad = width > total ? width - total : 0;
  if (!(flags & F_LEFT) && !(flags & F_ZERO)) sink_fill(s, ' ', pad);
  sink_put(s, prefix, (size_t)plen);
  if (!(flags & F_LEFT) && (flags & F_ZERO)) sink_fill(s, '0', pad);
  sink_fill(s, '0', zeros);
  sink_put(s, body, blen);
  if (flags & F_LEFT) sink_fill(s, ' ', pad);
}

static size_t fmt_unsigned(char *end, unsigned long long v, int base, int upper) {
  const char *digits = upper ? "0123456789ABCDEF" : "0123456789abcdef";
  char *p = end;
  do {
    *--p = digits[v % (unsigned)base];
    v /= (unsigned)base;
  } while (v);
  return (size_t)(end - p);
}

/* Splits Rust's "d.ddde-7" into mantissa length and exponent. */
static size_t split_exp(const char *s, size_t len, int *exp) {
  size_t m = 0;
  while (m < len && s[m] != 'e') m++;
  *exp = 0;
  if (m < len) {
    size_t i = m + 1;
    int neg = s[i] == '-';
    if (neg) i++;
    for (; i < len; i++) *exp = *exp * 10 + (s[i] - '0');
    if (neg) *exp = -*exp;
  }
  return m;
}

/* Appends C's exponent syntax: e+05. */
static size_t put_exp(char *p, int exp, char e) {
  size_t n = 0;
  char tmp[8];
  size_t k = fmt_unsigned(tmp + sizeof tmp, (unsigned)(exp < 0 ? -exp : exp), 10, 0);
  p[n++] = e;
  p[n++] = exp < 0 ? '-' : '+';
  if (k < 2) p[n++] = '0';
  memcpy(p + n, tmp + sizeof tmp - k, k);
  return n + k;
}

/* Removes trailing fractional zeros (and a bare point) from s[0..len). */
static size_t trim_zeros(const char *s, size_t len) {
  if (!memchr(s, '.', len)) return len;
  while (len && s[len - 1] == '0') len--;
  if (len && s[len - 1] == '.') len--;
  return len;
}

static size_t fmt_hex_float(char *out, double v, int prec, int upper, int alt) {
  const char *digits = upper ? "0123456789ABCDEF" : "0123456789abcdef";
  union { double d; uint64_t u; } bits = {v};
  uint64_t frac = bits.u & ((1ULL << 52) - 1);
  int bexp = (int)((bits.u >> 52) & 0x7ff);
  uint64_t lead = bexp ? 1 : 0;
  int exp = v == 0.0 ? 0 : (bexp ? bexp - 1023 : -1022);
  uint64_t mant = (lead << 52) | frac;
  int ndig = 13;
  size_t n = 0;

  if (prec >= 0 && prec < 13) {
    int shift = (13 - prec) * 4;
    uint64_t rem = mant & ((1ULL << shift) - 1), half = 1ULL << (shift - 1);
    mant >>= shift;
    if (rem > half || (rem == half && (mant & 1))) mant++;
    ndig = prec;
  } else if (prec < 0) {
    while (ndig > 0 && (mant & 0xf) == 0) mant >>= 4, ndig--;
  }
  out[n++] = '0';
  out[n++] = upper ? 'X' : 'x';
  out[n++] = digits[mant >> (ndig * 4)];
  if (ndig > 0 || prec > 0 || alt) out[n++] = '.';
  for (int i = ndig - 1; i >= 0; i--) out[n++] = digits[(mant >> (i * 4)) & 0xf];
  for (int i = ndig; i < prec; i++) out[n++] = '0';
  out[n++] = upper ? 'P' : 'p';
  out[n++] = exp < 0 ? '-' : '+';
  {
    char tmp[8];
    size_t k = fmt_unsigned(tmp + sizeof tmp, (unsigned)(exp < 0 ? -exp : exp), 10, 0);
    memcpy(out + n, tmp + sizeof tmp - k, k);
    n += k;
  }
  return n;
}

/* Formats a float conversion (e f g a, either case). */
static void fmt_float(struct sink *s, int flags, int width, int prec, char conv, double v) {
  char body[512];
  size_t blen;
  int upper = conv >= 'A' && conv <= 'Z';
  char lc = (char)(conv | 0x20);
  const char *sign = __builtin_signbit(v) ? "-" : (flags & F_PLUS) ? "+" : (flags & F_SPACE) ? " " : "";
  char prefix[4];

  if (v != v || v == HUGE_VAL || v == -HUGE_VAL) {
    const char *t = v != v ? (upper ? "NAN" : "nan") : (upper ? "INF" : "inf");
    if (v != v) sign = (flags & F_PLUS) ? "+" : (flags & F_SPACE) ? " " : "";
    emit(s, flags & ~F_ZERO, width, sign, 0, t, 3);
    return;
  }
  v = __builtin_fabs(v);

  if (lc == 'a') {
    blen = fmt_hex_float(body, v, prec, upper, flags & F_ALT);
    /* the "0x" goes before any zero padding */
    snprintf(prefix, sizeof prefix, "%s%c%c", sign, body[0], body[1]);
    emit(s, flags, width, prefix, 0, body + 2, blen - 2);
    return;
  }

  if (prec < 0) prec = 6;
  if (lc == 'g') {
    int p = prec == 0 ? 1 : prec, exp;
    char tmp[512];
    size_t tlen = lua_wasm_fmt_f64(tmp, sizeof tmp, v, p - 1, 'e');
    split_exp(tmp, tlen, &exp);
    if (p > exp && exp >= -4) {
      lc = 'f';
      prec = p - 1 - exp;
    } else {
      lc = 'e';
      prec = p - 1;
    }
    if (!(flags & F_ALT)) flags |= 0x100; /* trim trailing zeros */
  }

  if (lc == 'f') {
    blen = lua_wasm_fmt_f64(body, sizeof body - 2, v, prec, 'f');
    if (flags & 0x100) blen = trim_zeros(body, blen);
    else if ((flags & F_ALT) && prec == 0) body[blen++] = '.';
  } else {
    int exp;
    char tmp[512];
    size_t tlen = lua_wasm_fmt_f64(tmp, sizeof tmp - 16, v, prec, 'e');
    size_t m = split_exp(tmp, tlen, &exp);
    if (flags & 0x100) m = trim_zeros(tmp, m);
    else if ((flags & F_ALT) && prec == 0) tmp[m++] = '.';
    memcpy(body, tmp, m);
    blen = m + put_exp(body + m, exp, upper ? 'E' : 'e');
  }
  if (upper)
    for (size_t i = 0; i < blen; i++) body[i] = (char)toupper((unsigned char)body[i]);
  emit(s, flags, width, sign, 0, body, blen);
}

static int vformat(struct sink *s, const char *fmt, va_list ap) {
  for (; *fmt; fmt++) {
    if (*fmt != '%') {
      const char *lit = fmt;
      while (fmt[1] && fmt[1] != '%') fmt++;
      sink_put(s, lit, (size_t)(fmt - lit + 1));
      continue;
    }
    fmt++;

    int flags = 0, width = 0, prec = -1, len = 0; /* len: -2 hh, -1 h, 1 l, 2 ll/j/z/t */
    for (;; fmt++) {
      if (*fmt == '-') flags |= F_LEFT;
      else if (*fmt == '+') flags |= F_PLUS;
      else if (*fmt == ' ') flags |= F_SPACE;
      else if (*fmt == '#') flags |= F_ALT;
      else if (*fmt == '0') flags |= F_ZERO;
      else break;
    }
    if (*fmt == '*') {
      width = va_arg(ap, int);
      if (width < 0) flags |= F_LEFT, width = -width;
      fmt++;
    } else {
      while (isdigit((unsigned char)*fmt)) width = width * 10 + (*fmt++ - '0');
    }
    if (*fmt == '.') {
      fmt++;
      prec = 0;
      if (*fmt == '*') {
        prec = va_arg(ap, int);
        fmt++;
      } else {
        while (isdigit((unsigned char)*fmt)) prec = prec * 10 + (*fmt++ - '0');
      }
    }
    if (flags & F_LEFT) flags &= ~F_ZERO;
    for (;; fmt++) {
      if (*fmt == 'h') len--;
      else if (*fmt == 'l') len++;
      else if (*fmt == 'j' || *fmt == 'z' || *fmt == 't') len = 2;
      else if (*fmt == 'L') {}
      else break;
    }

    char conv = *fmt;
    char buf[72];
    char *end = buf + sizeof buf;
    switch (conv) {
      case 'd':
      case 'i': {
        long long v = len >= 2 ? va_arg(ap, long long) : len == 1 ? va_arg(ap, long) : va_arg(ap, int);
        if (len == -1) v = (short)v;
        if (len <= -2) v = (signed char)v;
        unsigned long long u = v < 0 ? 0ULL - (unsigned long long)v : (unsigned long long)v;
        const char *sign = v < 0 ? "-" : (flags & F_PLUS) ? "+" : (flags & F_SPACE) ? " " : "";
        size_t n = (prec == 0 && u == 0) ? 0 : fmt_unsigned(end, u, 10, 0);
        if (prec >= 0) flags &= ~F_ZERO;
        emit(s, flags, width, sign, prec > (int)n ? prec - (int)n : 0, end - n, n);
        break;
      }
      case 'u':
      case 'o':
      case 'x':
      case 'X': {
        unsigned long long u = len >= 2 ? va_arg(ap, unsigned long long)
                               : len == 1 ? va_arg(ap, unsigned long) : va_arg(ap, unsigned);
        if (len == -1) u = (unsigned short)u;
        if (len <= -2) u = (unsigned char)u;
        int base = conv == 'u' ? 10 : conv == 'o' ? 8 : 16;
        size_t n = (prec == 0 && u == 0) ? 0 : fmt_unsigned(end, u, base, conv == 'X');
        const char *prefix = "";
        int zeros = prec > (int)n ? prec - (int)n : 0;
        if ((flags & F_ALT) && conv == 'o' && zeros == 0 && (n == 0 || end[-(long)n] != '0')) zeros = 1;
        if ((flags & F_ALT) && base == 16 && u != 0) prefix = conv == 'X' ? "0X" : "0x";
        if (prec >= 0) flags &= ~F_ZERO;
        emit(s, flags, width, prefix, zeros, end - n, n);
        break;
      }
      case 'c': {
        char c = (char)va_arg(ap, int);
        emit(s, flags & ~F_ZERO, width, "", 0, &c, 1);
        break;
      }
      case 's': {
        const char *str = va_arg(ap, const char *);
        size_t n;
        if (!str) str = "(null)";
        if (prec >= 0) {
          const char *z = memchr(str, 0, (size_t)prec);
          n = z ? (size_t)(z - str) : (size_t)prec;
        } else {
          n = strlen(str);
        }
        emit(s, flags & ~F_ZERO, width, "", 0, str, n);
        break;
      }
      case 'p': {
        uintptr_t p = (uintptr_t)va_arg(ap, void *);
        size_t n = fmt_unsigned(end, p, 16, 0);
        emit(s, flags & ~F_ZERO, width, "0x", 0, end - n, n);
        break;
      }
      case 'e': case 'E': case 'f': case 'F': case 'g': case 'G': case 'a': case 'A':
        fmt_float(s, flags, width, prec, conv, va_arg(ap, double));
        break;
      case '%':
        sink_put(s, "%", 1);
        break;
      case '\0':
        fmt--; /* malformed trailing '%' */
        break;
      default: /* unknown conversion: print it as is */
        sink_put(s, fmt - 1, 2);
        break;
    }
  }
  return (int)s->len;
}

int vsnprintf(char *buf, size_t n, const char *fmt, va_list ap) {
  struct sink s = {buf, n, 0, NULL};
  int r = vformat(&s, fmt, ap);
  if (n) buf[s.len < n ? s.len : n - 1] = '\0';
  return r;
}

int snprintf(char *buf, size_t n, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vsnprintf(buf, n, fmt, ap);
  va_end(ap);
  return r;
}

int sprintf(char *buf, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vsnprintf(buf, (size_t)-1 / 2, fmt, ap);
  va_end(ap);
  return r;
}

int vfprintf(FILE *f, const char *fmt, va_list ap) {
  struct sink s = {NULL, 0, 0, f};
  return vformat(&s, fmt, ap);
}

int fprintf(FILE *f, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vfprintf(f, fmt, ap);
  va_end(ap);
  return r;
}

int printf(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vfprintf(stdout, fmt, ap);
  va_end(ap);
  return r;
}

/* --- stdio streams ------------------------------------------------------- */

struct lua_wasm_file {
  int fd;
  int error;
  int eof;
  int ungot; /* pushed-back char, or EOF */
};

static struct lua_wasm_file std_files[3] = {{0, 0, 0, EOF}, {1, 0, 0, EOF}, {2, 0, 0, EOF}};
FILE *const stdin = &std_files[0];
FILE *const stdout = &std_files[1];
FILE *const stderr = &std_files[2];

FILE *fopen(const char *path, const char *mode) {
  (void)path, (void)mode;
  errno = ENOENT;
  return NULL;
}

FILE *freopen(const char *path, const char *mode, FILE *f) {
  (void)path, (void)mode, (void)f;
  errno = ENOENT;
  return NULL;
}

FILE *tmpfile(void) {
  errno = ENOSYS;
  return NULL;
}

char *tmpnam(char *s) { (void)s; return NULL; }

int fclose(FILE *f) { return fflush(f); }

int fflush(FILE *f) {
  if (f == NULL) {
    lua_wasm_flush(1);
    lua_wasm_flush(2);
  } else if (f->fd != 0) {
    lua_wasm_flush(f->fd);
  }
  return 0;
}

int setvbuf(FILE *f, char *buf, int mode, size_t size) {
  (void)f, (void)buf, (void)mode, (void)size;
  return 0;
}

size_t fwrite(const void *ptr, size_t size, size_t n, FILE *f) {
  if (f->fd == 0) {
    f->error = 1;
    errno = EBADF;
    return 0;
  }
  lua_wasm_write(f->fd, ptr, size * n);
  return n;
}

int fputs(const char *str, FILE *f) { return fwrite(str, 1, strlen(str), f) ? 0 : EOF; }
int puts(const char *str) { return fputs(str, stdout) == EOF ? EOF : fputs("\n", stdout); }

int fputc(int c, FILE *f) {
  char ch = (char)c;
  return fwrite(&ch, 1, 1, f) ? (unsigned char)c : EOF;
}
int putc(int c, FILE *f) { return fputc(c, f); }
int putchar(int c) { return fputc(c, stdout); }

/* stdin is always at end of file. */
int fgetc(FILE *f) {
  if (f->ungot != EOF) {
    int c = f->ungot;
    f->ungot = EOF;
    return c;
  }
  f->eof = 1;
  return EOF;
}

int getc(FILE *f) { return fgetc(f); }

int ungetc(int c, FILE *f) {
  if (c == EOF) return EOF;
  f->ungot = (unsigned char)c;
  f->eof = 0;
  return f->ungot;
}

size_t fread(void *ptr, size_t size, size_t n, FILE *f) {
  unsigned char *p = ptr;
  size_t total = size * n, got = 0;
  int c;
  while (got < total && (c = fgetc(f)) != EOF) p[got++] = (unsigned char)c;
  return size ? got / size : 0;
}

char *fgets(char *str, int n, FILE *f) {
  int i = 0, c;
  while (i < n - 1 && (c = fgetc(f)) != EOF) {
    str[i++] = (char)c;
    if (c == '\n') break;
  }
  if (i == 0) return NULL;
  str[i] = '\0';
  return str;
}

int fseek(FILE *f, long off, int whence) {
  (void)f, (void)off, (void)whence;
  errno = EINVAL;
  return -1;
}

long ftell(FILE *f) {
  (void)f;
  errno = EINVAL;
  return -1;
}

int feof(FILE *f) { return f->eof; }
int ferror(FILE *f) { return f->error; }
void clearerr(FILE *f) { f->eof = f->error = 0; }

int remove(const char *path) { (void)path; errno = ENOSYS; return -1; }
int rename(const char *from, const char *to) { (void)from, (void)to; errno = ENOSYS; return -1; }

/* --- time.h (UTC only) --------------------------------------------------- */

time_t time(time_t *t) {
  time_t now = (time_t)(lua_wasm_time_ms() / 1000.0);
  if (t) *t = now;
  return now;
}

clock_t clock(void) { return (clock_t)(lua_wasm_clock_ms() * (CLOCKS_PER_SEC / 1000)); }

double difftime(time_t a, time_t b) { return (double)(a - b); }

/* Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant). */
static long long days_from_civil(long long y, int m, int d) {
  y -= m <= 2;
  long long era = (y >= 0 ? y : y - 399) / 400;
  long long yoe = y - era * 400;
  long long doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
  long long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  return era * 146097 + doe - 719468;
}

static void civil_from_days(long long z, long long *y, int *m, int *d) {
  z += 719468;
  long long era = (z >= 0 ? z : z - 146096) / 146097;
  long long doe = z - era * 146097;
  long long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  long long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  long long mp = (5 * doy + 2) / 153;
  *d = (int)(doy - (153 * mp + 2) / 5 + 1);
  *m = (int)(mp < 10 ? mp + 3 : mp - 9);
  *y = yoe + era * 400 + (*m <= 2);
}

static long long floor_div(long long a, long long b) {
  return a / b - ((a % b != 0) && ((a < 0) != (b < 0)));
}

static void fill_tm(time_t t, struct tm *tm) {
  long long days = floor_div(t, 86400), secs = t - days * 86400, y;
  int m, d;
  civil_from_days(days, &y, &m, &d);
  tm->tm_year = (int)(y - 1900);
  tm->tm_mon = m - 1;
  tm->tm_mday = d;
  tm->tm_hour = (int)(secs / 3600);
  tm->tm_min = (int)(secs / 60 % 60);
  tm->tm_sec = (int)(secs % 60);
  tm->tm_wday = (int)((days % 7 + 11) % 7); /* 1970-01-01 was a Thursday */
  tm->tm_yday = (int)(days - days_from_civil(y, 1, 1));
  tm->tm_isdst = 0;
}

struct tm *gmtime(const time_t *t) {
  static struct tm tm;
  fill_tm(*t, &tm);
  return &tm;
}

struct tm *localtime(const time_t *t) { return gmtime(t); }

time_t mktime(struct tm *tm) {
  long long mon = tm->tm_mon;
  long long year = 1900LL + tm->tm_year + floor_div(mon, 12);
  mon -= floor_div(mon, 12) * 12;
  time_t t = (days_from_civil(year, (int)mon + 1, 1) + tm->tm_mday - 1) * 86400LL +
             tm->tm_hour * 3600LL + tm->tm_min * 60LL + tm->tm_sec;
  fill_tm(t, tm);
  return t;
}

static const char *const day_names[] = {"Sunday", "Monday", "Tuesday", "Wednesday",
                                        "Thursday", "Friday", "Saturday"};
static const char *const month_names[] = {"January", "February", "March",     "April",
                                          "May",     "June",     "July",      "August",
                                          "September", "October", "November", "December"};

size_t strftime(char *out, size_t max, const char *fmt, const struct tm *tm) {
  struct sink s = {out, max, 0, NULL};
  char tmp[64];
  for (; *fmt; fmt++) {
    if (*fmt != '%') {
      sink_put(&s, fmt, 1);
      continue;
    }
    fmt++;
    if (*fmt == 'E' || *fmt == 'O') fmt++; /* alternative forms: same output */
    int n = 0;
    switch (*fmt) {
      case 'a': n = snprintf(tmp, sizeof tmp, "%.3s", day_names[tm->tm_wday % 7]); break;
      case 'A': n = snprintf(tmp, sizeof tmp, "%s", day_names[tm->tm_wday % 7]); break;
      case 'b': case 'h': n = snprintf(tmp, sizeof tmp, "%.3s", month_names[tm->tm_mon % 12]); break;
      case 'B': n = snprintf(tmp, sizeof tmp, "%s", month_names[tm->tm_mon % 12]); break;
      case 'c':
        n = snprintf(tmp, sizeof tmp, "%.3s %.3s %2d %02d:%02d:%02d %d", day_names[tm->tm_wday % 7],
                     month_names[tm->tm_mon % 12], tm->tm_mday, tm->tm_hour, tm->tm_min,
                     tm->tm_sec, tm->tm_year + 1900);
        break;
      case 'C': n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_year + 1900) / 100); break;
      case 'd': n = snprintf(tmp, sizeof tmp, "%02d", tm->tm_mday); break;
      case 'D':
        n = snprintf(tmp, sizeof tmp, "%02d/%02d/%02d", tm->tm_mon + 1, tm->tm_mday,
                     (tm->tm_year + 1900) % 100);
        break;
      case 'e': n = snprintf(tmp, sizeof tmp, "%2d", tm->tm_mday); break;
      case 'F':
        n = snprintf(tmp, sizeof tmp, "%d-%02d-%02d", tm->tm_year + 1900, tm->tm_mon + 1, tm->tm_mday);
        break;
      case 'g': case 'y': n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_year + 1900) % 100); break;
      case 'G': case 'Y': n = snprintf(tmp, sizeof tmp, "%d", tm->tm_year + 1900); break;
      case 'H': n = snprintf(tmp, sizeof tmp, "%02d", tm->tm_hour); break;
      case 'I': n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_hour + 11) % 12 + 1); break;
      case 'j': n = snprintf(tmp, sizeof tmp, "%03d", tm->tm_yday + 1); break;
      case 'm': n = snprintf(tmp, sizeof tmp, "%02d", tm->tm_mon + 1); break;
      case 'M': n = snprintf(tmp, sizeof tmp, "%02d", tm->tm_min); break;
      case 'n': n = snprintf(tmp, sizeof tmp, "\n"); break;
      case 'p': n = snprintf(tmp, sizeof tmp, "%s", tm->tm_hour < 12 ? "AM" : "PM"); break;
      case 'r':
        n = snprintf(tmp, sizeof tmp, "%02d:%02d:%02d %s", (tm->tm_hour + 11) % 12 + 1, tm->tm_min,
                     tm->tm_sec, tm->tm_hour < 12 ? "AM" : "PM");
        break;
      case 'R': n = snprintf(tmp, sizeof tmp, "%02d:%02d", tm->tm_hour, tm->tm_min); break;
      case 'S': n = snprintf(tmp, sizeof tmp, "%02d", tm->tm_sec); break;
      case 't': n = snprintf(tmp, sizeof tmp, "\t"); break;
      case 'T': case 'X':
        n = snprintf(tmp, sizeof tmp, "%02d:%02d:%02d", tm->tm_hour, tm->tm_min, tm->tm_sec);
        break;
      case 'u': n = snprintf(tmp, sizeof tmp, "%d", tm->tm_wday ? tm->tm_wday : 7); break;
      case 'w': n = snprintf(tmp, sizeof tmp, "%d", tm->tm_wday); break;
      case 'U': n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_yday + 7 - tm->tm_wday) / 7); break;
      case 'W':
        n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_yday + 7 - (tm->tm_wday + 6) % 7) / 7);
        break;
      case 'V': n = snprintf(tmp, sizeof tmp, "%02d", (tm->tm_yday + 7 - (tm->tm_wday + 6) % 7) / 7 + 1); break;
      case 'x':
        n = snprintf(tmp, sizeof tmp, "%02d/%02d/%02d", tm->tm_mon + 1, tm->tm_mday,
                     (tm->tm_year + 1900) % 100);
        break;
      case 'z': n = snprintf(tmp, sizeof tmp, "+0000"); break;
      case 'Z': n = snprintf(tmp, sizeof tmp, "UTC"); break;
      case '%': n = snprintf(tmp, sizeof tmp, "%%"); break;
      case '\0': fmt--; break;
      default: break;
    }
    sink_put(&s, tmp, (size_t)n);
  }
  if (s.len >= max) return 0;
  out[s.len] = '\0';
  return s.len;
}
