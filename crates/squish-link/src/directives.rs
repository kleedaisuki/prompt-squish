//! Portable side-channel emitted by archive-target evaluation.
use squish_ir::{
    DecodeError, Digest, DigestAlgorithm, FrameId, ImportId, LinkedOpRef, ObjectDigest, OpId,
    OriginId, PackageInstanceId, QualifiedOriginRef, SourceKey, SubstitutionKind, SubstitutionStep,
    TraceRef,
};

/// An archive member request, owned by the unit defining the operation, never its caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveDirective {
    /// Copy complete frozen bytes into the finished pack.
    Asset {
        /// Logical source owning the operation and its relative resource path.
        source: SourceKey,
        /// Resource path relative to the defining source.
        path: String,
        /// Validated destination path in the finished ZIP.
        name: String,
        /// Complete call-site and definition-site provenance.
        trace: TraceRef,
    },
    /// Compile an entry selected by a frozen import binding into a finished pack member.
    Include {
        /// Logical source owning the operation and its relative resource path.
        source: SourceKey,
        /// Frozen import edge selecting an independently linked entry.
        import: ImportId,
        /// Resource path relative to the defining source.
        path: String,
        /// Validated destination path in the finished ZIP.
        name: String,
        /// Complete call-site and definition-site provenance.
        trace: TraceRef,
    },
}

/// Encodes ordered archive directives for project-local persistent action reuse.
#[must_use]
pub fn encode_archive_directives(directives: &[ArchiveDirective]) -> Vec<u8> {
    let mut w = Writer(b"XSAD\x01".to_vec());
    w.num(directives.len() as u64);
    for directive in directives {
        let (source, path, name, trace) = match directive {
            ArchiveDirective::Asset {
                source,
                path,
                name,
                trace,
            } => {
                w.0.push(0);
                (source, path, name, trace)
            }
            ArchiveDirective::Include {
                source,
                import,
                path,
                name,
                trace,
            } => {
                w.0.push(1);
                w.num(import.0.into());
                (source, path, name, trace)
            }
        };
        w.source(source);
        w.string(path);
        w.string(name);
        w.trace(trace);
    }
    w.0
}

/// Decodes bounded canonical directives; rejects unknown tags and trailing bytes.
///
/// Trace frame IDs are checked against the companion expansion trace by the manager.
pub fn decode_archive_directives(bytes: &[u8]) -> Result<Vec<ArchiveDirective>, DecodeError> {
    if !bytes.starts_with(b"XSAD\x01") {
        return Err(DecodeError::NonCanonical("archive directives magic"));
    }
    let mut r = Reader { bytes, at: 5 };
    let count = r.count()?;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = r.byte()?;
        let import = if tag == 1 {
            Some(ImportId(r.id()?))
        } else {
            None
        };
        if tag > 1 {
            return Err(DecodeError::NonCanonical("archive directive tag"));
        }
        let source = r.source()?;
        let path = r.string()?;
        let name = r.string()?;
        let trace = r.trace()?;
        result.push(match import {
            Some(import) => ArchiveDirective::Include {
                source,
                import,
                path,
                name,
                trace,
            },
            None => ArchiveDirective::Asset {
                source,
                path,
                name,
                trace,
            },
        });
    }
    if r.at != bytes.len() {
        return Err(DecodeError::NonCanonical(
            "archive directive trailing bytes",
        ));
    }
    Ok(result)
}
struct Writer(Vec<u8>);
impl Writer {
    fn num(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn string(&mut self, s: &str) {
        self.num(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn source(&mut self, s: &SourceKey) {
        match s {
            SourceKey::AdHoc { uri } => {
                self.0.push(0);
                self.string(uri)
            }
            SourceKey::Project { package, path } => {
                self.0.push(1);
                self.num(package.source_kind.into());
                self.string(&package.canonical_source);
                self.string(&package.package_name);
                self.string(&package.exact_revision);
                self.num(path.len() as u64);
                for p in path {
                    self.string(p);
                }
            }
        }
    }
    fn origin(&mut self, o: &QualifiedOriginRef) {
        self.num(o.object.0.algorithm as u64);
        self.0.extend_from_slice(&o.object.0.bytes);
        self.num(o.local.0.into());
    }
    fn optional(&mut self, o: &Option<QualifiedOriginRef>) {
        self.0.push(u8::from(o.is_some()));
        if let Some(o) = o {
            self.origin(o);
        }
    }
    fn trace(&mut self, t: &TraceRef) {
        self.num(t.producer_op.unit_slot.into());
        self.num(t.producer_op.op.0.into());
        self.num(t.frame.0.into());
        self.origin(&t.definition_origin);
        self.optional(&t.call_origin);
        self.num(t.substitution_chain.len() as u64);
        for s in &t.substitution_chain {
            self.0.push(match s.kind {
                SubstitutionKind::SlotFill => 0,
                SubstitutionKind::ScalarBody => 1,
                SubstitutionKind::InsertScalar => 2,
            });
            self.origin(&s.origin);
        }
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|n| *n <= self.bytes.len())
            .ok_or(DecodeError::NonCanonical("truncated archive directives"))?;
        let b = &self.bytes[self.at..end];
        self.at = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn num(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn id(&mut self) -> Result<u32, DecodeError> {
        u32::try_from(self.num()?).map_err(|_| DecodeError::NonCanonical("directive ID overflow"))
    }
    fn count(&mut self) -> Result<usize, DecodeError> {
        let n = usize::try_from(self.num()?)
            .map_err(|_| DecodeError::NonCanonical("directive length overflow"))?;
        if n > self.bytes.len().saturating_sub(self.at) {
            return Err(DecodeError::NonCanonical(
                "directive length exceeds payload",
            ));
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| DecodeError::NonCanonical("directive UTF-8"))
    }
    fn source(&mut self) -> Result<SourceKey, DecodeError> {
        Ok(match self.byte()? {
            0 => SourceKey::AdHoc {
                uri: self.string()?,
            },
            1 => {
                let source_kind = u16::try_from(self.num()?)
                    .map_err(|_| DecodeError::NonCanonical("directive source kind"))?;
                let canonical_source = self.string()?;
                let package_name = self.string()?;
                let exact_revision = self.string()?;
                let n = self.count()?;
                let mut path = Vec::with_capacity(n);
                for _ in 0..n {
                    path.push(self.string()?);
                }
                SourceKey::Project {
                    package: PackageInstanceId {
                        source_kind,
                        canonical_source,
                        package_name,
                        exact_revision,
                    },
                    path,
                }
            }
            _ => return Err(DecodeError::NonCanonical("directive source tag")),
        })
    }
    fn origin(&mut self) -> Result<QualifiedOriginRef, DecodeError> {
        let algorithm = match self.num()? {
            1 => DigestAlgorithm::Sha256,
            2 => DigestAlgorithm::Blake3,
            _ => return Err(DecodeError::NonCanonical("directive digest algorithm")),
        };
        let bytes = self.take(32)?.try_into().unwrap();
        let local = OriginId(self.id()?);
        Ok(QualifiedOriginRef {
            object: ObjectDigest(Digest { algorithm, bytes }),
            local,
        })
    }
    fn optional(&mut self) -> Result<Option<QualifiedOriginRef>, DecodeError> {
        match self.byte()? {
            0 => Ok(None),
            1 => Ok(Some(self.origin()?)),
            _ => Err(DecodeError::NonCanonical("directive optional tag")),
        }
    }
    fn trace(&mut self) -> Result<TraceRef, DecodeError> {
        let producer_op = LinkedOpRef {
            unit_slot: self.id()?,
            op: OpId(self.id()?),
        };
        let frame = FrameId(self.id()?);
        let definition_origin = self.origin()?;
        let call_origin = self.optional()?;
        let n = self.count()?;
        let mut substitution_chain = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = match self.byte()? {
                0 => SubstitutionKind::SlotFill,
                1 => SubstitutionKind::ScalarBody,
                2 => SubstitutionKind::InsertScalar,
                _ => return Err(DecodeError::NonCanonical("directive substitution tag")),
            };
            substitution_chain.push(SubstitutionStep {
                kind,
                origin: self.origin()?,
            });
        }
        Ok(TraceRef {
            producer_op,
            frame,
            definition_origin,
            call_origin,
            substitution_chain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directive() -> ArchiveDirective {
        let origin = QualifiedOriginRef {
            object: ObjectDigest::of(b"provider-object"),
            local: OriginId(7),
        };
        ArchiveDirective::Include {
            source: SourceKey::Project {
                package: PackageInstanceId {
                    source_kind: 5,
                    canonical_source: "sopack:immutable".into(),
                    package_name: "library".into(),
                    exact_revision: "abc".into(),
                },
                path: vec!["src".into(), "module.xml".into()],
            },
            import: ImportId(12),
            path: "child.xml".into(),
            name: "child.prompt".into(),
            trace: TraceRef {
                producer_op: LinkedOpRef {
                    unit_slot: 2,
                    op: OpId(10),
                },
                frame: FrameId(5),
                definition_origin: origin.clone(),
                call_origin: Some(origin.clone()),
                substitution_chain: vec![SubstitutionStep {
                    kind: SubstitutionKind::SlotFill,
                    origin,
                }],
            },
        }
    }
    #[test]
    fn persistent_directives_preserve_defining_owner_and_trace() {
        let original = vec![directive()];
        let bytes = encode_archive_directives(&original);
        assert_eq!(decode_archive_directives(&bytes).unwrap(), original);
        assert_eq!(
            encode_archive_directives(&decode_archive_directives(&bytes).unwrap()),
            bytes
        );
    }
    #[test]
    fn bounded_decode_rejects_every_truncation_and_trailing_data() {
        let bytes = encode_archive_directives(&[directive()]);
        for end in 0..bytes.len() {
            assert!(decode_archive_directives(&bytes[..end]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_archive_directives(&trailing).is_err());
        let mut oversized = b"XSAD\x01".to_vec();
        oversized.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_archive_directives(&oversized).is_err());
    }
}
