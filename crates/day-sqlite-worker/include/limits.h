// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The hermetic <limits.h> for the vendored engine build — same rationale as the sibling
// stdint.h: gcc's limits.h unconditionally #include_next's the host libc's, and on glibc that
// header reads <features.h>, which resolves to the vendored musl copy and breaks. Everything
// here is derived from gcc/clang predefines, so the include chain ends in this file.
#ifndef DAY_SQLITE_WORKER_LIMITS_H
#define DAY_SQLITE_WORKER_LIMITS_H

#define CHAR_BIT __CHAR_BIT__
#define SCHAR_MAX __SCHAR_MAX__
#define SCHAR_MIN (-__SCHAR_MAX__ - 1)
#define UCHAR_MAX (__SCHAR_MAX__ * 2 + 1)
#ifdef __CHAR_UNSIGNED__
#define CHAR_MIN 0
#define CHAR_MAX UCHAR_MAX
#else
#define CHAR_MIN SCHAR_MIN
#define CHAR_MAX SCHAR_MAX
#endif
#define MB_LEN_MAX 4

#define SHRT_MAX __SHRT_MAX__
#define SHRT_MIN (-__SHRT_MAX__ - 1)
#define USHRT_MAX (__SHRT_MAX__ * 2 + 1)
#define INT_MAX __INT_MAX__
#define INT_MIN (-__INT_MAX__ - 1)
#define UINT_MAX (__INT_MAX__ * 2U + 1U)
#define LONG_MAX __LONG_MAX__
#define LONG_MIN (-__LONG_MAX__ - 1L)
#define ULONG_MAX (__LONG_MAX__ * 2UL + 1UL)
#define LLONG_MAX __LONG_LONG_MAX__
#define LLONG_MIN (-__LONG_LONG_MAX__ - 1LL)
#define ULLONG_MAX (__LONG_LONG_MAX__ * 2ULL + 1ULL)

#endif
