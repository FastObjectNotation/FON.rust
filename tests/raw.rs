use fon::types::{FonCollection, FonValue};
use fon::{deserialize_line, serialize_to_string, DeserializeOptions, RawData};


fn pack(bytes: &[u8]) -> String {
    let mut raw: RawData = RawData::from_bytes(bytes.to_vec());
    raw.pack();
    raw.encoded().to_string()
}


fn unpack(encoded: &str) -> Result<Vec<u8>, fon::FonError> {
    let mut raw: RawData = RawData::from_encoded(encoded.to_string());
    raw.unpack()?;
    Ok(raw.data().to_vec())
}


// Deterministic bytes covering the whole 0..=255 range.
fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(97).wrapping_add(13) % 256) as u8).collect()
}


#[test]
fn every_length_round_trips() {
    for len in 1..=64 {
        let data: Vec<u8> = sample(len);
        assert_eq!(unpack(&pack(&data)).unwrap(), data, "length {len}");
    }
}


#[test]
fn full_groups_ending_in_a_padding_digit_round_trip() {
    // Each of these 4-byte values encodes to a group whose last character is '1', '2'
    // or '3', the same characters that mark padding.
    for value in [1u32, 2, 3, 86, 7226, 85_002, 85_003, u32::MAX - 84] {
        let data: Vec<u8> = value.to_be_bytes().to_vec();
        let encoded: String = pack(&data);
        assert_eq!(encoded.len(), 5);
        assert!(
            matches!(encoded.as_bytes()[4], b'1'..=b'3'),
            "{value} encodes to {encoded}"
        );
        assert_eq!(unpack(&encoded).unwrap(), data, "{value} via {encoded}");

        let mut doubled: Vec<u8> = data.clone();
        doubled.extend_from_slice(&data);
        assert_eq!(unpack(&pack(&doubled)).unwrap(), doubled, "{value} twice");
    }
}


#[test]
fn groups_seen_in_the_wild_decode() {
    assert_eq!(unpack("009f3").unwrap().len(), 4);
    assert_eq!(unpack("00101").unwrap(), vec![0, 0, 0x1C, 0x3A]);
    assert_eq!(unpack("00102").unwrap(), vec![0, 0, 0x1C, 0x3B]);
}


#[test]
fn malformed_input_is_an_error_not_a_panic() {
    for encoded in ["1", "2", "3", "0", "ab", "abc", "abcd", "000000", "0000004", "000000000"] {
        assert!(unpack(encoded).is_err(), "{encoded:?} accepted");
    }
    // Five characters that decode past u32::MAX.
    assert!(unpack("#####").is_err());
}


#[test]
fn raw_values_survive_a_serialize_round_trip() {
    let opts: DeserializeOptions = DeserializeOptions { unpack_raw: true, ..DeserializeOptions::default() };
    for len in [1usize, 2, 3, 4, 5, 8, 4096] {
        let data: Vec<u8> = sample(len);
        let mut c: FonCollection = FonCollection::new();
        c.add("blob", FonValue::Raw(Box::new(RawData::from_bytes(data.clone()))));
        let line: String = serialize_to_string(&c);
        let back: FonCollection = deserialize_line(line.as_bytes(), &opts).unwrap();
        match back.get("blob") {
            Some(FonValue::Raw(raw)) => assert_eq!(raw.data(), data.as_slice(), "length {len}"),
            _ => panic!("expected raw value"),
        }
    }
}
