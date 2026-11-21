//! `memcmp` and `bcmp` for the static musl builds. musl's `memcmp` compares
//! one byte at a time, which makes the stages that compare many keys markedly
//! slower on musl than on glibc; these compare eight bytes at a time.
//!
//! On musl targets the two are exported under their C names, so the static
//! link takes them in place of musl's, for the Rust code and the bundled C
//! libraries alike. Elsewhere they are ordinary functions, tested the same.
//! Neither may compare slices: that would call `memcmp`, which is itself.
//!
//! Every read is volatile, so the compiler may neither drop nor merge one,
//! and so cannot recognise either loop as a comparison and lower it back
//! into a call to `memcmp` or `bcmp`. Each word is still one 8-byte load,
//! and callers may still inline them. (`#![no_builtins]` would also prevent
//! the lowering, but it keeps the crate out of link-time optimisation, so
//! no caller could inline them: the tests stage ran 9–14% slower.)

use std::ffi::{c_int, c_void};

/// C's `memcmp`: compares `n` bytes as unsigned values and returns a negative
/// number, zero or a positive number as the first differing byte of `a` is
/// less than, equal to or greater than that of `b`.
///
/// # Safety
///
/// `a` and `b` must each be valid for reads of `n` bytes; with `n` of zero
/// neither is read.
#[cfg_attr(target_env = "musl", unsafe(no_mangle))]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    let (a, b) = (a.cast::<u8>(), b.cast::<u8>());
    let mut i = 0;
    // SAFETY: every read is of bytes below `n`, which the caller vouches for.
    unsafe {
        while i + 8 <= n {
            // Big-endian, so the first byte is the most significant.
            let x = u64::from_be_bytes(a.add(i).cast::<[u8; 8]>().read_volatile());
            let y = u64::from_be_bytes(b.add(i).cast::<[u8; 8]>().read_volatile());
            if x != y {
                return if x < y { -1 } else { 1 };
            }
            i += 8;
        }
        while i < n {
            let (x, y) = (a.add(i).read_volatile(), b.add(i).read_volatile());
            if x != y {
                return c_int::from(x) - c_int::from(y);
            }
            i += 1;
        }
    }
    0
}

/// C's `bcmp`: zero if and only if the `n` bytes of `a` and `b` are equal.
///
/// # Safety
///
/// As for [`memcmp`].
#[cfg_attr(target_env = "musl", unsafe(no_mangle))]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    // SAFETY: the caller's guarantee is memcmp's.
    unsafe { memcmp(a, b, n) }
}
