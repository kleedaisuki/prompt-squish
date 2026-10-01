//! 算法带标签的摘要类型。 / Algorithm-tagged digest types.

use core::fmt;
use sha2::Digest as _;

/// 持久摘要算法。 / Persistent digest algorithm.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum DigestAlgorithm {
    /// IR 语义与制品身份使用 SHA-256。 / SHA-256 for IR semantics and artifact identity.
    Sha256 = 1,
    /// CAS 实现可使用 BLAKE3，但不得冒充 IR 摘要。 / CAS may use BLAKE3, never as IR identity.
    Blake3 = 2,
}

/// 具有显式算法的 256 位摘要。 / A 256-bit digest with an explicit algorithm.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub bytes: [u8; 32],
}

impl Digest {
    /// 计算规范的领域分隔 SHA-256。 / Computes canonical domain-separated SHA-256.
    #[must_use]
    pub fn sha256(domain: &str, payload: &[u8]) -> Self {
        Self::sha256_parts(domain, [payload])
    }

    /// Hashes a domain-separated sequence without concatenating its payload.
    pub(crate) fn sha256_parts<'a>(
        domain: &str,
        parts: impl IntoIterator<Item = &'a [u8]>,
    ) -> Self {
        let mut hash = DomainHasher::new(domain);
        for part in parts {
            hash.update(part);
        }
        hash.finish()
    }

    /// 返回小写十六进制。 / Returns lowercase hexadecimal.
    #[must_use]
    pub fn hex(self) -> String {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut o = String::with_capacity(64);
        for b in self.bytes {
            o.push(H[(b >> 4) as usize] as char);
            o.push(H[(b & 15) as usize] as char);
        }
        o
    }
}
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}:{}", self.algorithm, self.hex())
    }
}

macro_rules! digest_newtype { ($($name:ident=>$domain:literal),+$(,)?)=>{$(
    #[doc=concat!("类型安全摘要。 / Type-safe digest for `",$domain,"`.")]
    #[derive(Clone,Copy,Debug,Eq,Hash,Ord,PartialEq,PartialOrd)] pub struct $name(pub Digest);
    impl $name { #[must_use] pub fn of(bytes:&[u8])->Self{Self(Digest::sha256($domain,bytes))} }
)+};}
digest_newtype! {SourceDigest=>"source",SemanticUnitDigest=>"unit",DebugDigest=>"debug",ObjectDigest=>"object",LinkedImageDigest=>"linked",DocumentDigest=>"document",ArtifactDigest=>"artifact"}

/// Streaming domain-separated SHA-256 shared by canonical persistence writers.
pub(crate) struct DomainHasher(sha2::Sha256);
impl DomainHasher {
    pub(crate) fn new(domain: &str) -> Self {
        let mut hash = sha2::Sha256::new();
        hash.update(b"xmlsquish\0");
        hash.update(domain.as_bytes());
        hash.update([0]);
        Self(hash)
    }
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    pub(crate) fn finish(self) -> Digest {
        Digest {
            algorithm: DigestAlgorithm::Sha256,
            bytes: self.0.finalize().into(),
        }
    }
}

#[cfg(test)]
fn sha256(input: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(input).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_separated_sha256_matches_external_vector() {
        assert_eq!(
            Digest {
                algorithm: DigestAlgorithm::Sha256,
                bytes: sha256(b"abc")
            }
            .hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Digest::sha256("source", b"").hex(),
            "19e870d3e8bf9ae077cf6e49dffcb9ccd53eea3b73c03184b755b8f54bd7abd5"
        );
    }
    #[test]
    fn segmented_hash_preserves_domain_and_all_block_boundaries() {
        for length in [0, 1, 55, 56, 63, 64, 65, 127, 128, 129, 1024, 4096] {
            let bytes: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
            let mut framed = b"xmlsquish\0boundary\0".to_vec();
            framed.extend_from_slice(&bytes);
            let expected: [u8; 32] = sha2::Sha256::digest(&framed).into();
            for width in [1, 7, 31, 64, 65] {
                assert_eq!(
                    Digest::sha256_parts("boundary", bytes.chunks(width)).bytes,
                    expected,
                    "length={length}, width={width}"
                );
            }
        }
        assert_eq!(
            Digest {
                algorithm: DigestAlgorithm::Sha256,
                bytes: sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
            }
            .hex(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
