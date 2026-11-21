//! The id that git gives a file's content, worked out here from the bytes
//! that were read: the hash of `blob <length>`, a NUL and the bytes, by
//! SHA-1 or, in a repository that names its objects so, by SHA-256.
//!
//! Git is not asked. It would read the file again, and what it then read
//! need not be what was read here. The hashes themselves are not written
//! here: they are those of the `sha1` and `sha2` crates. What is here is
//! what git puts before the content, and it is held to git's own ids in
//! the tests that run the binary.

use sha1::Sha1;
use sha2::{Digest, Sha256};

/// How a repository names its objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Sha1,
    Sha256,
}

impl Format {
    /// The format that ids of the length of `id` are of, if they are of one.
    pub fn of(id: &str) -> Option<Format> {
        match id.len() {
            40 => Some(Format::Sha1),
            64 => Some(Format::Sha256),
            _ => None,
        }
    }

    /// The id of the blob that `content` is.
    pub fn blob(self, content: &[u8]) -> String {
        let header = format!("blob {}\0", content.len());
        let message = [header.as_bytes(), content];
        match self {
            Format::Sha1 => digest::<Sha1>(message),
            Format::Sha256 => digest::<Sha256>(message),
        }
    }
}

/// The hash of a message that is two runs of bytes, one after the other,
/// in hexadecimal digits.
fn digest<D: Digest>(message: [&[u8]; 2]) -> String {
    let mut hash = D::new();
    for bytes in message {
        hash.update(bytes);
    }
    let hash = hash.finalize();
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha_1_of(message: &[u8]) -> String {
        digest::<Sha1>([message, b""])
    }

    fn sha_256_of(message: &[u8]) -> String {
        digest::<Sha256>([message, b""])
    }

    /// The messages of FIPS 180-4's examples: none, one block, and one
    /// that needs a block more for its length.
    const TWO_BLOCKS: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";

    #[test]
    fn sha_1_gives_the_published_hashes() {
        assert_eq!(sha_1_of(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha_1_of(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha_1_of(TWO_BLOCKS),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            sha_1_of(&[b'a'; 1_000_000]),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn sha_256_gives_the_published_hashes() {
        assert_eq!(
            sha_256_of(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha_256_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha_256_of(TWO_BLOCKS),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha_256_of(&[b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn a_message_hashes_the_same_however_it_is_cut_in_two() {
        let message: Vec<u8> = (0..200u8).collect();
        let whole = sha_1_of(&message);

        // Every cut: the header and the content of a blob are two runs.
        for cut in 0..=message.len() {
            let (first, second) = message.split_at(cut);
            assert_eq!(digest::<Sha1>([first, second]), whole, "cut at {cut}");
        }
        for length in 50..70 {
            let (first, second) = message[..length].split_at(length / 2);
            let cut = digest::<Sha256>([first, second]);
            assert_eq!(cut, sha_256_of(&message[..length]));
        }
    }

    #[test]
    fn a_blob_has_the_id_that_git_gives_it() {
        // `git hash-object` of no content and of `hello` with a line end.
        assert_eq!(
            Format::Sha1.blob(b""),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        assert_eq!(
            Format::Sha1.blob(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            Format::Sha256.blob(b""),
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813"
        );
        assert_eq!(Format::of(&Format::Sha1.blob(b"")), Some(Format::Sha1));
        assert_eq!(Format::of(&Format::Sha256.blob(b"")), Some(Format::Sha256));
        assert_eq!(Format::of("abc"), None);
    }
}
