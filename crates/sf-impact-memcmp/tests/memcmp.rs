//! The word-at-a-time `memcmp` and `bcmp` that the static musl builds link in
//! place of musl's byte loop. Each is held to a byte-at-a-time reference:
//! every length to 64 at every pair of alignments, then random and
//! overlapping buffers. On musl the tests also show that the C symbols are
//! these functions, so every comparison in the test binary goes through them.

use sf_impact_memcmp::{bcmp, memcmp};

/// What C requires of `memcmp`: the sign of the first differing byte pair,
/// compared as unsigned bytes, or 0 if there is none.
fn reference(a: &[u8], b: &[u8]) -> i32 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

/// Both functions on `a` and `b`, checked against the reference.
fn check(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    let expected = reference(a, b);
    let (n, pa, pb) = (a.len(), a.as_ptr().cast(), b.as_ptr().cast());
    let got = unsafe { memcmp(pa, pb, n) }.signum();
    assert_eq!(got, expected, "memcmp of {a:?} and {b:?}");
    let equal = unsafe { bcmp(pa, pb, n) } == 0;
    assert_eq!(equal, expected == 0, "bcmp of {a:?} and {b:?}");
}

#[repr(align(8))]
struct Aligned([u8; 80]);

/// A buffer whose bytes all differ from their neighbours, so a comparison
/// that reads the wrong word or the wrong byte order sees a difference.
fn patterned() -> Aligned {
    let mut buffer = Aligned([0; 80]);
    for (i, byte) in buffer.0.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    buffer
}

#[test]
fn every_length_to_64_at_every_alignment() {
    // Byte pairs either side of 0x80, so a signed comparison is caught.
    let pairs = [
        (0x00, 0x01),
        (0x01, 0x00),
        (0x7f, 0x80),
        (0x80, 0x7f),
        (0x00, 0xff),
    ];
    for n in 0..=64 {
        for offset_a in 0..8 {
            for offset_b in 0..8 {
                let mut a = patterned();
                let mut b = patterned();
                // b's copy of the pattern starts where a's does, at any alignment.
                b.0[offset_b..offset_b + n].copy_from_slice(&a.0[offset_a..offset_a + n]);
                check(&a.0[offset_a..][..n], &b.0[offset_b..][..n]);
                for at in 0..n {
                    for (x, y) in pairs {
                        let (sa, sb) = (a.0[offset_a + at], b.0[offset_b + at]);
                        a.0[offset_a + at] = x;
                        b.0[offset_b + at] = y;
                        check(&a.0[offset_a..][..n], &b.0[offset_b..][..n]);
                        // A later difference the other way does not count.
                        if at + 1 < n {
                            let (la, lb) = (a.0[offset_a + at + 1], b.0[offset_b + at + 1]);
                            a.0[offset_a + at + 1] = y;
                            b.0[offset_b + at + 1] = x;
                            check(&a.0[offset_a..][..n], &b.0[offset_b..][..n]);
                            a.0[offset_a + at + 1] = la;
                            b.0[offset_b + at + 1] = lb;
                        }
                        a.0[offset_a + at] = sa;
                        b.0[offset_b + at] = sb;
                    }
                }
            }
        }
    }
}

#[test]
fn a_length_of_zero_reads_nothing() {
    let null = std::ptr::null();
    assert_eq!(unsafe { memcmp(null, null, 0) }, 0);
    assert_eq!(unsafe { bcmp(null, null, 0) }, 0);
}

/// SplitMix64, so that a failure can be replayed from its seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn random_buffers_agree_with_the_reference() {
    let mut random = Random(0x5f1d_2026_1118);
    // Few distinct bytes, so that long equal runs are common.
    let alphabet = [0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff];
    for _ in 0..20_000 {
        let n = match random.below(4) {
            0 => random.below(16),
            1 => random.below(256),
            _ => random.below(4096),
        };
        let a: Vec<u8> = (0..n).map(|_| alphabet[random.below(6)]).collect();
        let mut b = a.clone();
        for _ in 0..random.below(3) {
            if n > 0 {
                b[random.below(n)] = alphabet[random.below(6)];
            }
        }
        let (offset_a, offset_b) = (random.below(8), random.below(8));
        let mut sa = vec![0xaa; offset_a];
        sa.extend_from_slice(&a);
        let mut sb = vec![0x55; offset_b];
        sb.extend_from_slice(&b);
        check(&sa[offset_a..], &sb[offset_b..]);
    }
}

#[test]
fn overlapping_ranges_of_one_buffer() {
    let mut random = Random(0x0b5e_55ed);
    let buffer: Vec<u8> = (0..2048)
        .map(|_| [0x00, 0x80, 0xff][random.below(3)])
        .collect();
    for _ in 0..20_000 {
        let n = random.below(1024);
        let (i, j) = (random.below(2048 - n), random.below(2048 - n));
        check(&buffer[i..i + n], &buffer[j..j + n]);
        check(&buffer[i..i + n], &buffer[i..i + n]);
    }
}

#[cfg(target_env = "musl")]
#[test]
fn the_c_symbols_are_these_functions() {
    use std::ffi::{c_int, c_void};
    unsafe extern "C" {
        #[link_name = "memcmp"]
        fn c_memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int;
        #[link_name = "bcmp"]
        fn c_bcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int;
    }
    assert_eq!(c_memcmp as *const (), memcmp as *const ());
    assert_eq!(c_bcmp as *const (), bcmp as *const ());
}

/// Bytes that end where an unreadable page starts, so that a read past the
/// last of them faults and kills the test.
#[cfg(target_os = "linux")]
struct Guarded {
    base: *mut u8,
    page: usize,
}

#[cfg(target_os = "linux")]
impl Guarded {
    fn new() -> Self {
        use std::ffi::{c_int, c_long, c_void};
        unsafe extern "C" {
            fn sysconf(name: c_int) -> c_long;
            fn mmap(
                addr: *mut c_void,
                len: usize,
                prot: c_int,
                flags: c_int,
                fd: c_int,
                off: i64,
            ) -> *mut c_void;
            fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
        }
        // Linux's values, the same on x86_64 and aarch64, glibc and musl.
        const SC_PAGESIZE: c_int = 30;
        const PROT_NONE: c_int = 0;
        const PROT_READ_WRITE: c_int = 3;
        const MAP_PRIVATE_ANONYMOUS: c_int = 0x02 | 0x20;
        unsafe {
            let page = sysconf(SC_PAGESIZE) as usize;
            let base = mmap(
                std::ptr::null_mut(),
                2 * page,
                PROT_READ_WRITE,
                MAP_PRIVATE_ANONYMOUS,
                -1,
                0,
            );
            assert_ne!(base as isize, -1, "mmap");
            assert_eq!(
                mprotect(base.cast::<u8>().add(page).cast(), page, PROT_NONE),
                0
            );
            Guarded {
                base: base.cast(),
                page,
            }
        }
    }

    /// The last `n` readable bytes, set to `bytes`.
    fn at_edge(&mut self, bytes: &[u8]) -> &[u8] {
        unsafe {
            let start = self.base.add(self.page - bytes.len());
            let edge = std::slice::from_raw_parts_mut(start, bytes.len());
            edge.copy_from_slice(bytes);
            edge
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn nothing_past_the_last_byte_is_read() {
    let (mut a_page, mut b_page) = (Guarded::new(), Guarded::new());
    let mut lengths: Vec<usize> = (0..=24).collect();
    lengths.extend([31, 32, 33, 39, 40, 41, 63, 64, 65]);
    for n in lengths {
        let a: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37)).collect();
        // Equal, and differing at the first, a middle and the last byte.
        let mut variants = vec![a.clone()];
        for at in [0, n / 2, n.saturating_sub(1)]
            .into_iter()
            .filter(|_| n > 0)
        {
            let mut b = a.clone();
            b[at] ^= 0x80;
            variants.push(b);
        }
        for b in variants {
            // Each at the edge alone, the other on the heap, then both.
            check(a_page.at_edge(&a), &b);
            check(&a, b_page.at_edge(&b));
            check(a_page.at_edge(&a), b_page.at_edge(&b));
        }
    }
}
