use super::*;

fn checksum(bytes: &[u8]) -> [u8; 16] {
    use std::hash::{DefaultHasher, Hasher};
    let mut first = DefaultHasher::new();
    first.write(bytes);
    let mut second = DefaultHasher::new();
    second.write_u8(1);
    second.write(bytes);
    let mut result = [0; 16];
    result[..8].copy_from_slice(&first.finish().to_le_bytes());
    result[8..].copy_from_slice(&second.finish().to_le_bytes());
    result
}

fn binding() -> CacheBinding {
    CacheBinding {
        compiler: b"compiler/cache-v1".to_vec(),
        target: b"target/layout".to_vec(),
        options: b"optimization/codegen/debug".to_vec(),
        cgu: b"crate.cgu".to_vec(),
    }
}

fn manifest() -> ArtifactManifest {
    ArtifactManifest::new(binding(), b"canonical module".to_vec(), b"object".to_vec())
}

#[test]
fn manifest_round_trip_and_all_bindings() {
    let original = manifest();
    let decoded = ArtifactManifest::decode(&original.encode(checksum).unwrap(), checksum).unwrap();
    assert_eq!(original, decoded);
    assert_eq!(decoded.binding(), &binding());
    assert_eq!(decoded.canonical_input(), b"canonical module");
    assert_eq!(decoded.artifact(), b"object");
    assert!(decoded.matches(&binding(), b"canonical module", b"object"));
    for component in 0..4 {
        let mut changed = binding();
        let fields =
            [&mut changed.compiler, &mut changed.target, &mut changed.options, &mut changed.cgu];
        fields.into_iter().nth(component).unwrap().push(0);
        assert!(!decoded.matches(&changed, b"canonical module", b"object"));
    }
    assert!(!decoded.matches(&binding(), b"changed module", b"object"));
    assert!(!decoded.matches(&binding(), b"canonical module", b"changed object"));
}

#[test]
fn manifest_rejects_truncation_version_lengths_and_trailing_data() {
    let bytes = manifest().encode(checksum).unwrap();
    for end in 0..bytes.len() {
        assert!(ArtifactManifest::decode(&bytes[..end], checksum).is_none(), "length {end}");
    }
    let mut changed = bytes.clone();
    changed[7] += 1;
    let payload_len = changed.len() - CHECKSUM_SIZE;
    let digest = checksum(&changed[..payload_len]);
    changed[payload_len..].copy_from_slice(&digest);
    assert!(ArtifactManifest::decode(&changed, checksum).is_none());
    let mut changed = bytes.clone();
    changed[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    let payload_len = changed.len() - CHECKSUM_SIZE;
    let digest = checksum(&changed[..payload_len]);
    changed[payload_len..].copy_from_slice(&digest);
    assert!(ArtifactManifest::decode(&changed, checksum).is_none());
    let mut changed = bytes;
    changed.push(0);
    let payload_len = changed.len() - CHECKSUM_SIZE;
    let digest = checksum(&changed[..payload_len]);
    changed[payload_len..].copy_from_slice(&digest);
    assert!(ArtifactManifest::decode(&changed, checksum).is_none());
}

#[test]
fn corrupt_payload_cannot_match_original_artifact_or_input() {
    let original = manifest();
    let encoded = original.encode(checksum).unwrap();
    for offset in 0..encoded.len() {
        let mut corrupt = encoded.clone();
        corrupt[offset] ^= 1;
        assert!(ArtifactManifest::decode(&corrupt, checksum).is_none());
    }
}

fn region(identity: u8, source: &str, text: &str) -> SourceRegion {
    let start = source.find(text).unwrap();
    SourceRegion::from_source([identity; 16], [42; 16], source, start, start + text.len()).unwrap()
}

#[test]
fn positive_negative_and_repeated_moves_preserve_utf8_crlf_positions() {
    let text = "fn café() {\r\n    println!(\"é\");\r\n}";
    let original = format!("// é\r\n{text}\r\n");
    let moved = format!("// é\r\n\r\n\r\n{text}\r\n");
    let old = region(1, &original, text);
    let new = region(1, &moved, text);
    let forward = RelocationMap::new(&[old.clone()], &[new.clone()]).unwrap();
    let reverse = RelocationMap::new(&[new.clone()], &[old.clone()]).unwrap();
    for line in old.start_line..=old.end_line {
        assert_eq!(forward.translate_line(old.identity, line), Some(line + 2));
        assert_eq!(reverse.translate_line(old.identity, line + 2), Some(line));
    }
    for byte in old.start_byte..=old.end_byte().unwrap() {
        let translated = forward.translate_byte(old.identity, byte).unwrap();
        assert_eq!(translated, byte + 4);
        assert_eq!(reverse.translate_byte(old.identity, translated), Some(byte));
    }
    let no_op = RelocationMap::new(&[new.clone()], &[new.clone()]).unwrap();
    assert_eq!(no_op.translate_line(new.identity, new.start_line), Some(new.start_line));
    assert_eq!(forward.translate_byte(old.identity, old.start_byte - 1), None);
    assert_eq!(forward.translate_byte(old.identity, old.end_byte().unwrap() + 1), None);
    assert_eq!(forward.translate_line(old.identity, old.start_line - 1), None);
    assert_eq!(forward.translate_line(old.identity, old.end_line + 1), None);
    assert_eq!(forward.translate_line([0; 16], old.start_line), None);
}

#[test]
fn multiple_entities_have_independent_signed_shifts() {
    let old = [region(1, "a\n\n\nb", "a"), region(2, "a\n\n\nb", "b")];
    let new = [region(1, "\na\nb", "a"), region(2, "\na\nb", "b")];
    let map = RelocationMap::new(&old, &new).unwrap();
    assert_eq!(map.translate_line([1; 16], 1), Some(2));
    assert_eq!(map.translate_line([2; 16], 4), Some(3));
}

#[test]
fn changes_ambiguity_cross_file_and_column_changes_are_misses() {
    let old = region(1, "abc", "abc");
    let changed = region(1, "abd", "abd");
    assert!(RelocationMap::new(&[old.clone()], &[changed]).is_none());
    assert!(RelocationMap::new(&[old.clone(), old.clone()], &[old.clone()]).is_none());
    let overlapping = region(2, "abc", "bc");
    assert!(
        RelocationMap::new(&[old.clone(), overlapping.clone()], &[old.clone(), overlapping])
            .is_none()
    );
    let mut other_file = old.clone();
    other_file.file = [43; 16];
    assert!(RelocationMap::new(&[old.clone()], &[other_file]).is_none());
    let indented = region(1, " abc", "abc");
    assert!(RelocationMap::new(&[old.clone()], &[indented]).is_none());
    assert!(RelocationMap::new(&[old.clone()], &[]).is_none());
    assert!(RelocationMap::new(&[], &[old]).is_none());
}

#[test]
fn invalid_ranges_and_non_character_boundaries_are_misses() {
    assert!(SourceRegion::from_source([1; 16], [2; 16], "é", 1, 2).is_none());
    assert!(SourceRegion::from_source([1; 16], [2; 16], "é", 0, 1).is_none());
    assert!(SourceRegion::from_source([1; 16], [2; 16], "abc", 2, 1).is_none());
    assert!(SourceRegion::from_source([1; 16], [2; 16], "abc", 0, 4).is_none());
    assert!(SourceRegion::from_source([1; 16], [2; 16], "abc", 0, 0).is_none());
}

#[test]
fn translation_and_region_bounds_fail_closed_on_overflow() {
    let old = region(1, "ab", "ab");
    let mut new = old.clone();
    new.start_byte = u64::MAX;
    new.start_line = u32::MAX;
    let map = RelocationMap { regions: BTreeMap::from([(old.identity, (old.clone(), new))]) };
    assert_eq!(map.translate_byte(old.identity, old.start_byte + 1), None);
    let mut old_multiline = old.clone();
    old_multiline.end_line += 1;
    let mut new = old.clone();
    new.start_line = u32::MAX;
    let map = RelocationMap { regions: BTreeMap::from([(old.identity, (old_multiline, new))]) };
    assert_eq!(map.translate_line(old.identity, old.start_line + 1), None);
}
