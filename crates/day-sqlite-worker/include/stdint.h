// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The hermetic <stdint.h> for the vendored engine build (build.rs puts this directory first on
// the include path). Every type and limit comes from the compiler's own predefines, so this
// header is complete in itself and the include never falls through to a host libc. That
// fall-through is not hypothetical: gcc's and clang's stdint.h are self-contained only in
// freestanding builds — hosted, they #include_next the libc's, and on glibc that header reads
// <features.h>, which this build shadows with the vendored musl copy (which defines no
// __GLIBC_USE), aborting the compile on every Linux host. Only gcc/clang dialects build this
// tree (build.rs rejects MSVC targets), so the __INT*_TYPE__ predefine family is always present.
#ifndef DAY_SQLITE_WORKER_STDINT_H
#define DAY_SQLITE_WORKER_STDINT_H

typedef __INT8_TYPE__ int8_t;
typedef __INT16_TYPE__ int16_t;
typedef __INT32_TYPE__ int32_t;
typedef __INT64_TYPE__ int64_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __UINT64_TYPE__ uint64_t;

typedef __INT_LEAST8_TYPE__ int_least8_t;
typedef __INT_LEAST16_TYPE__ int_least16_t;
typedef __INT_LEAST32_TYPE__ int_least32_t;
typedef __INT_LEAST64_TYPE__ int_least64_t;
typedef __UINT_LEAST8_TYPE__ uint_least8_t;
typedef __UINT_LEAST16_TYPE__ uint_least16_t;
typedef __UINT_LEAST32_TYPE__ uint_least32_t;
typedef __UINT_LEAST64_TYPE__ uint_least64_t;

typedef __INT_FAST8_TYPE__ int_fast8_t;
typedef __INT_FAST16_TYPE__ int_fast16_t;
typedef __INT_FAST32_TYPE__ int_fast32_t;
typedef __INT_FAST64_TYPE__ int_fast64_t;
typedef __UINT_FAST8_TYPE__ uint_fast8_t;
typedef __UINT_FAST16_TYPE__ uint_fast16_t;
typedef __UINT_FAST32_TYPE__ uint_fast32_t;
typedef __UINT_FAST64_TYPE__ uint_fast64_t;

typedef __INTPTR_TYPE__ intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;
typedef __INTMAX_TYPE__ intmax_t;
typedef __UINTMAX_TYPE__ uintmax_t;

#define INT8_MIN (-__INT8_MAX__ - 1)
#define INT16_MIN (-__INT16_MAX__ - 1)
#define INT32_MIN (-__INT32_MAX__ - 1)
#define INT64_MIN (-__INT64_MAX__ - 1)
#define INT8_MAX __INT8_MAX__
#define INT16_MAX __INT16_MAX__
#define INT32_MAX __INT32_MAX__
#define INT64_MAX __INT64_MAX__
#define UINT8_MAX __UINT8_MAX__
#define UINT16_MAX __UINT16_MAX__
#define UINT32_MAX __UINT32_MAX__
#define UINT64_MAX __UINT64_MAX__

#define INT_LEAST8_MIN (-__INT_LEAST8_MAX__ - 1)
#define INT_LEAST16_MIN (-__INT_LEAST16_MAX__ - 1)
#define INT_LEAST32_MIN (-__INT_LEAST32_MAX__ - 1)
#define INT_LEAST64_MIN (-__INT_LEAST64_MAX__ - 1)
#define INT_LEAST8_MAX __INT_LEAST8_MAX__
#define INT_LEAST16_MAX __INT_LEAST16_MAX__
#define INT_LEAST32_MAX __INT_LEAST32_MAX__
#define INT_LEAST64_MAX __INT_LEAST64_MAX__
#define UINT_LEAST8_MAX __UINT_LEAST8_MAX__
#define UINT_LEAST16_MAX __UINT_LEAST16_MAX__
#define UINT_LEAST32_MAX __UINT_LEAST32_MAX__
#define UINT_LEAST64_MAX __UINT_LEAST64_MAX__

#define INT_FAST8_MIN (-__INT_FAST8_MAX__ - 1)
#define INT_FAST16_MIN (-__INT_FAST16_MAX__ - 1)
#define INT_FAST32_MIN (-__INT_FAST32_MAX__ - 1)
#define INT_FAST64_MIN (-__INT_FAST64_MAX__ - 1)
#define INT_FAST8_MAX __INT_FAST8_MAX__
#define INT_FAST16_MAX __INT_FAST16_MAX__
#define INT_FAST32_MAX __INT_FAST32_MAX__
#define INT_FAST64_MAX __INT_FAST64_MAX__
#define UINT_FAST8_MAX __UINT_FAST8_MAX__
#define UINT_FAST16_MAX __UINT_FAST16_MAX__
#define UINT_FAST32_MAX __UINT_FAST32_MAX__
#define UINT_FAST64_MAX __UINT_FAST64_MAX__

#define INTPTR_MIN (-__INTPTR_MAX__ - 1)
#define INTPTR_MAX __INTPTR_MAX__
#define UINTPTR_MAX __UINTPTR_MAX__
#define INTMAX_MIN (-__INTMAX_MAX__ - 1)
#define INTMAX_MAX __INTMAX_MAX__
#define UINTMAX_MAX __UINTMAX_MAX__

#define PTRDIFF_MIN (-__PTRDIFF_MAX__ - 1)
#define PTRDIFF_MAX __PTRDIFF_MAX__
#define SIZE_MAX __SIZE_MAX__
#define SIG_ATOMIC_MIN (-__SIG_ATOMIC_MAX__ - 1)
#define SIG_ATOMIC_MAX __SIG_ATOMIC_MAX__
#define WCHAR_MAX __WCHAR_MAX__
#ifdef __WCHAR_MIN__
#define WCHAR_MIN __WCHAR_MIN__
#else
#define WCHAR_MIN (-__WCHAR_MAX__ - 1)
#endif
#define WINT_MAX __WINT_MAX__
#ifdef __WINT_MIN__
#define WINT_MIN __WINT_MIN__
#else
#define WINT_MIN (-__WINT_MAX__ - 1)
#endif

// The constant macros spell their suffixes here rather than deferring to the gcc-style
// __INT64_C()/__UINT64_C() family, which is the ONE part of the predefine set that is not
// universal: gcc defines it, clang only started to, and the clang 18 that Ubuntu 24.04 ships
// (what the Linux CI runners compile this with) does not. There the deferral expanded
// `UINT64_C(x)` into a call to an undeclared `__UINT64_C`, which failed every wasm build of
// the amalgamation. `LL`/`ULL` is musl's spelling and satisfies the standard, which asks only
// for a constant of at least the named width.
#define INT8_C(c) c
#define INT16_C(c) c
#define INT32_C(c) c
#define INT64_C(c) c##LL
#define UINT8_C(c) c
#define UINT16_C(c) c
#define UINT32_C(c) c##U
#define UINT64_C(c) c##ULL
#define INTMAX_C(c) c##LL
#define UINTMAX_C(c) c##ULL

#endif
