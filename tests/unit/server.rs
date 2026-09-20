use super::*;

#[test]
fn decoded_body_limit_and_invalid_json() {
    assert!(decode_body(Bytes::from_static(b"NaN"), false).is_err());
    assert!(decode_body(Bytes::from_static(b"bad"), true).is_err());
    let encoded = zstd::encode_all(vec![b' '; MAX_BODY + 1].as_slice(), 1).unwrap();
    assert_eq!(
        decode_body(encoded.into(), true).unwrap_err().0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let encoded = zstd::encode_all(b"{\"input\":\"hello\"}".as_slice(), 1).unwrap();
    assert_eq!(decode_body(encoded.into(), true).unwrap()["input"], "hello");
}
