//! 算法带标签的摘要类型。 / Algorithm-tagged digest types.

use core::fmt;

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
        let mut bytes = Vec::with_capacity(12 + domain.len() + payload.len());
        bytes.extend_from_slice(b"xmlsquish\0");
        bytes.extend_from_slice(domain.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(payload);
        Self {
            algorithm: DigestAlgorithm::Sha256,
            bytes: sha256(&bytes),
        }
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

// 固定分块写法兼容项目 MSRV；新编译器建议的 `as_chunks` 稳定得更晚。
// Fixed chunk iteration preserves the project MSRV; `as_chunks` stabilized later.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let n = (input.len() as u64).wrapping_mul(8);
    let mut d = input.to_vec();
    d.push(128);
    while d.len() % 64 != 56 {
        d.push(0)
    }
    d.extend_from_slice(&n.to_be_bytes());
    for block in d.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]])
        }
        for i in 16..64 {
            let a = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let b = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(a)
                .wrapping_add(w[i - 7])
                .wrapping_add(b)
        }
        let [mut a, mut b, mut c, mut dd, mut e, mut f, mut g, mut z] = h;
        for i in 0..64 {
            let s = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t = z
                .wrapping_add(s)
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let q = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            z = g;
            g = f;
            f = e;
            e = dd.wrapping_add(t);
            dd = c;
            c = b;
            b = a;
            a = t.wrapping_add(q)
        }
        for (x, y) in h.iter_mut().zip([a, b, c, dd, e, f, g, z]) {
            *x = x.wrapping_add(y)
        }
    }
    let mut o = [0; 32];
    for (c, v) in o.chunks_exact_mut(4).zip(h) {
        c.copy_from_slice(&v.to_be_bytes())
    }
    o
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
}
