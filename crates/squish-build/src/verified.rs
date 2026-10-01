//! Immutable buffers whose digest has been established once at an ownership boundary.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use squish_protocol::DigestAlgorithm;

use crate::{ActionRecord, ContentDigest};

/// An immutable BLAKE3-verified buffer, independent of its persistent storage location.
///
/// Private fields prevent an adapter from pairing arbitrary bytes with a claimed digest.
/// Constructing a handle hashes its owned/shared bytes exactly once; cloning and `bytes()`
/// do not hash or copy the payload. The explicit `shared_bytes()` compatibility view may
/// lazily copy an owned input into an Arc slice. A handle proves byte identity, not durable CAS publication.
///
/// # Example
/// ```
/// use squish_build::VerifiedBlob;
/// let blob = VerifiedBlob::from_owned(b"immutable input".to_vec());
/// let shared = blob.clone();
/// assert_eq!(shared.digest(), blob.digest());
/// assert_eq!(shared.bytes(), b"immutable input");
/// ```
#[derive(Clone)]
pub struct VerifiedBlob {
    /// Shared digest authority as well as payload, avoiding a digest Vec allocation per clone.
    inner: Arc<VerifiedBlobData>,
}

/// Immutable payload ownership, retaining an acquired Vec allocation without conversion.
enum VerifiedBytes {
    /// The exact owned input allocation; moving it into the handle copies no payload bytes.
    Owned(Vec<u8>),
    /// An already-shared immutable allocation supplied by a caller.
    Shared(Arc<[u8]>),
}

impl VerifiedBytes {
    /// Borrows either representation without changing ownership or materializing a copy.
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

/// The immutable digest/payload relationship minted by the hashing constructors.
struct VerifiedBlobData {
    /// Digest computed from the exact immutable byte allocation.
    digest: ContentDigest,
    /// Shared read-only authority; no mutable accessor or unchecked constructor exists.
    bytes: VerifiedBytes,
    /// Materialized only when the explicit Arc-slice compatibility accessor is requested.
    shared: OnceLock<Arc<[u8]>>,
}

impl PartialEq for VerifiedBlob {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
            || (self.digest() == other.digest() && self.bytes() == other.bytes())
    }
}

impl Eq for VerifiedBlob {}

impl fmt::Debug for VerifiedBlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Binary assets and generated prompts are not appropriate debug-log payloads.
        formatter
            .debug_struct("VerifiedBlob")
            .field("digest", &self.inner.digest)
            .field("size", &self.bytes().len())
            .finish()
    }
}

impl VerifiedBlob {
    /// Takes ownership of a complete buffer and hashes its immutable representation once.
    #[must_use]
    pub fn from_owned(bytes: Vec<u8>) -> Self {
        Self::from_payload(VerifiedBytes::Owned(bytes))
    }

    /// Hashes an immutable shared allocation once, retaining the allocation without a copy.
    #[must_use]
    pub fn from_shared(bytes: Arc<[u8]>) -> Self {
        Self::from_payload(VerifiedBytes::Shared(bytes))
    }

    /// Hashes exact owned/shared bytes before sealing their identity in the shared handle.
    fn from_payload(bytes: VerifiedBytes) -> Self {
        let hash = blake3::hash(bytes.as_slice());
        let digest = ContentDigest::new(DigestAlgorithm::Blake3, hash.as_bytes().to_vec())
            .expect("BLAKE3 produces the protocol's canonical digest length");
        Self {
            inner: Arc::new(VerifiedBlobData {
                digest,
                bytes,
                shared: OnceLock::new(),
            }),
        }
    }

    /// Returns the digest of the exact bytes owned by this handle.
    #[must_use]
    pub fn digest(&self) -> &ContentDigest {
        &self.inner.digest
    }

    /// Borrows the immutable bytes without allocation, copying, or hashing.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.inner.bytes.as_slice()
    }

    /// Returns an Arc-slice compatibility view, materializing it lazily for owned Vec inputs.
    ///
    /// Shared inputs preserve their original Arc allocation. Owned inputs copy their payload
    /// once, only when this accessor is explicitly called; subsequent calls reuse that view.
    /// Verified compiler/publication paths should retain this handle and use `bytes()` instead.
    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        match &self.inner.bytes {
            VerifiedBytes::Shared(bytes) => Arc::clone(bytes),
            VerifiedBytes::Owned(bytes) => Arc::clone(
                self.inner
                    .shared
                    .get_or_init(|| Arc::from(bytes.as_slice())),
            ),
        }
    }
}

/// A verified action manifest with caller-owned handles in output-declaration order.
///
/// The handles remain valid if an invocation cache evicts an entry or has a zero-byte budget.
/// Consumers should match each handle's digest and length to its output before hydration;
/// public fields allow injected runtime adapters, whose manifest claims still need checking.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAction {
    /// Persistent action identity and declared output names, kinds, sizes, and digests.
    pub record: ActionRecord,
    /// Verified buffers corresponding one-for-one to `record.outputs`.
    pub blobs: Vec<VerifiedBlob>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_handles_share_bytes_and_cannot_change_the_digest() {
        let bytes: Arc<[u8]> = Arc::from(&b"stable"[..]);
        let blob = VerifiedBlob::from_shared(Arc::clone(&bytes));
        let clone = blob.clone();
        assert!(Arc::ptr_eq(&blob.inner, &clone.inner));
        assert!(Arc::ptr_eq(&bytes, &blob.shared_bytes()));
        assert!(Arc::ptr_eq(&blob.shared_bytes(), &clone.shared_bytes()));
        assert_eq!(blob.digest().bytes(), blake3::hash(b"stable").as_bytes());
        assert_eq!(VerifiedBlob::from_owned(b"stable".to_vec()), blob);
    }
    #[test]
    fn owned_constructor_preserves_the_exact_vec_payload_allocation() {
        let bytes = vec![0x5a; 1024 * 1024];
        let pointer = bytes.as_ptr();
        let blob = VerifiedBlob::from_owned(bytes);
        let clone = blob.clone();
        assert_eq!(blob.bytes().as_ptr(), pointer);
        assert_eq!(clone.bytes().as_ptr(), pointer);
        assert!(blob.inner.shared.get().is_none());
        assert!(Arc::ptr_eq(&blob.inner, &clone.inner));
        assert_eq!(blob.digest().bytes(), blake3::hash(blob.bytes()).as_bytes());
    }

    #[test]
    fn compatibility_materialization_is_lazy_and_not_part_of_byte_identity() {
        let owned = VerifiedBlob::from_owned(b"same identity".to_vec());
        let shared = VerifiedBlob::from_shared(Arc::from(&b"same identity"[..]));
        assert_eq!(owned, shared);
        assert!(owned.inner.shared.get().is_none());
        let first = owned.shared_bytes();
        let second = owned.shared_bytes();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(owned, shared);
    }
}
