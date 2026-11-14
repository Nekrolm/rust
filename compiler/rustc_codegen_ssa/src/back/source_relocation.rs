use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"RSRLOC\0\x01";
const CHECKSUM_SIZE: usize = 16;
const MAX_MANIFEST_SIZE: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheBinding {
    pub compiler: Vec<u8>,
    pub target: Vec<u8>,
    pub options: Vec<u8>,
    pub cgu: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactManifest {
    binding: CacheBinding,
    canonical_input: Vec<u8>,
    artifact: Vec<u8>,
}

impl ArtifactManifest {
    pub fn new(binding: CacheBinding, canonical_input: Vec<u8>, artifact: Vec<u8>) -> Self {
        Self { binding, canonical_input, artifact }
    }

    pub fn binding(&self) -> &CacheBinding {
        &self.binding
    }

    pub fn canonical_input(&self) -> &[u8] {
        &self.canonical_input
    }

    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }

    pub fn matches(&self, binding: &CacheBinding, canonical_input: &[u8], artifact: &[u8]) -> bool {
        &self.binding == binding
            && self.canonical_input == canonical_input
            && self.artifact == artifact
    }

    // The caller supplies rustc's cache checksum without coupling the placement primitives
    // to a compiler crate. This detects accidental cache corruption, not hostile input.
    // Equality checks still compare the complete input bytes.
    pub fn encode(&self, checksum: impl FnOnce(&[u8]) -> [u8; CHECKSUM_SIZE]) -> Option<Vec<u8>> {
        let fields = [
            &self.binding.compiler,
            &self.binding.target,
            &self.binding.options,
            &self.binding.cgu,
            &self.canonical_input,
            &self.artifact,
        ];
        let size = fields.iter().try_fold(MAGIC.len() + CHECKSUM_SIZE, |size, field| {
            size.checked_add(8)?.checked_add(field.len())
        })?;
        if size > MAX_MANIFEST_SIZE {
            return None;
        }
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(MAGIC);
        for field in fields {
            bytes.extend_from_slice(&u64::try_from(field.len()).ok()?.to_le_bytes());
            bytes.extend_from_slice(field);
        }
        bytes.extend_from_slice(&checksum(&bytes));
        Some(bytes)
    }

    pub fn decode(
        bytes: &[u8],
        checksum: impl FnOnce(&[u8]) -> [u8; CHECKSUM_SIZE],
    ) -> Option<Self> {
        if bytes.len() > MAX_MANIFEST_SIZE || !bytes.starts_with(MAGIC) {
            return None;
        }
        let payload_len = bytes.len().checked_sub(CHECKSUM_SIZE)?;
        let (bytes, expected_checksum) = bytes.split_at(payload_len);
        if checksum(bytes) != expected_checksum || !bytes.starts_with(MAGIC) {
            return None;
        }
        let mut remaining = &bytes[MAGIC.len()..];
        fn field(remaining: &mut &[u8]) -> Option<Vec<u8>> {
            let (size, rest) = remaining.split_at_checked(8)?;
            let size = usize::try_from(u64::from_le_bytes(size.try_into().ok()?)).ok()?;
            let (value, rest) = rest.split_at_checked(size)?;
            *remaining = rest;
            Some(value.to_vec())
        }
        let binding = CacheBinding {
            compiler: field(&mut remaining)?,
            target: field(&mut remaining)?,
            options: field(&mut remaining)?,
            cgu: field(&mut remaining)?,
        };
        let manifest = Self {
            binding,
            canonical_input: field(&mut remaining)?,
            artifact: field(&mut remaining)?,
        };
        remaining.is_empty().then_some(manifest)
    }
}

pub type StableSourceId = [u8; 16];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRegion {
    identity: StableSourceId,
    file: StableSourceId,
    start_byte: u64,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    content: Vec<u8>,
}

impl SourceRegion {
    // Callers must supply stable identities for original, unexpanded source. Text
    // equality alone cannot establish macro hygiene or semantic equivalence.
    pub fn from_source(
        identity: StableSourceId,
        file: StableSourceId,
        source: &str,
        start: usize,
        end: usize,
    ) -> Option<Self> {
        let prefix = source.get(..start)?;
        let content = source.get(start..end)?;
        if content.is_empty() {
            return None;
        }
        let start_line =
            u32::try_from(prefix.bytes().filter(|&b| b == b'\n').count()).ok()?.checked_add(1)?;
        let start_column = u32::try_from(prefix.rsplit('\n').next()?.chars().count()).ok()?;
        let end_line = start_line
            .checked_add(u32::try_from(content.bytes().filter(|&b| b == b'\n').count()).ok()?)?;
        Some(Self {
            identity,
            file,
            start_byte: u64::try_from(start).ok()?,
            start_line,
            start_column,
            end_line,
            content: content.as_bytes().to_vec(),
        })
    }

    fn end_byte(&self) -> Option<u64> {
        self.start_byte.checked_add(u64::try_from(self.content.len()).ok()?)
    }
}

#[derive(Clone, Debug)]
// This validates placement alone. The backend must independently prove unchanged
// non-debug inputs and reject expansion provenance it cannot map to these regions.
pub struct RelocationMap {
    regions: BTreeMap<StableSourceId, (SourceRegion, SourceRegion)>,
}

impl RelocationMap {
    pub fn new(previous: &[SourceRegion], current: &[SourceRegion]) -> Option<Self> {
        fn index(regions: &[SourceRegion]) -> Option<BTreeMap<StableSourceId, SourceRegion>> {
            let mut indexed = BTreeMap::new();
            for region in regions {
                if indexed.insert(region.identity, region.clone()).is_some() {
                    return None;
                }
            }
            let mut ordered: Vec<_> = regions.iter().collect();
            ordered.sort_by_key(|region| (region.file, region.start_byte));
            for pair in ordered.windows(2) {
                if pair[0].file == pair[1].file && pair[0].end_byte()? > pair[1].start_byte {
                    return None;
                }
            }
            Some(indexed)
        }
        let mut current = index(current)?;
        let previous = index(previous)?;
        if previous.len() != current.len() {
            return None;
        }
        let mut regions = BTreeMap::new();
        for (identity, old) in previous {
            let new = current.remove(&identity)?;
            if old.file != new.file
                || old.content != new.content
                || old.start_column != new.start_column
            {
                return None;
            }
            regions.insert(identity, (old, new));
        }
        Some(Self { regions })
    }

    pub fn translate_byte(&self, identity: StableSourceId, byte: u64) -> Option<u64> {
        let (old, new) = self.regions.get(&identity)?;
        if byte > old.end_byte()? {
            return None;
        }
        new.start_byte.checked_add(byte.checked_sub(old.start_byte)?)
    }

    pub fn translate_line(&self, identity: StableSourceId, line: u32) -> Option<u32> {
        let (old, new) = self.regions.get(&identity)?;
        if line > old.end_line {
            return None;
        }
        new.start_line.checked_add(line.checked_sub(old.start_line)?)
    }
}

#[cfg(test)]
#[path = "source_relocation/tests.rs"]
mod tests;
