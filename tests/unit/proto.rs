use super::*;

#[test]
fn measured_response_tags_and_presence() {
    let response = ChatResponse::decode(&b"\x1a\x02hi\x28\x00\x32\x02\x30\x00"[..]).unwrap();
    assert_eq!(response.delta_text, "hi");
    assert_eq!(response.stop_reason, Some(0));
    assert_eq!(
        response.delta_tool_calls[0].is_custom_tool_call,
        Some(false)
    );
    assert_eq!(ChatResponse::decode(&b""[..]).unwrap().stop_reason, None);
}

#[test]
fn malformed_protobuf_rejected() {
    for input in [&b"\x1a\x08hi"[..], &b"\x1a\x01\xff"[..], &b"\x80"[..]] {
        assert!(ChatResponse::decode(input).is_err());
    }
}

#[test]
fn official_image_field_numbers_encode_exactly() {
    let message = ChatMessage {
        images: vec![ImageData {
            base64_data: "AQID".into(),
            mime_type: "image/png".into(),
            caption: String::new(),
        }],
        ..Default::default()
    };
    assert_eq!(
        message.encode_to_vec(),
        b"\x52\x11\x0a\x04AQID\x12\x09image/png"
    );
}
