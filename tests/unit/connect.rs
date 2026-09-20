use super::*;
use std::io::Write;

fn gzip(value: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(value).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn fragmented_coalesced() {
    let data = [frame(b"first", 0), frame(b"second", 0), frame(b"{}", 2)].concat();
    for size in [1, 2, 5, 13, 65536] {
        let mut parser = ConnectParser::new(100, false);
        let mut output = Vec::new();
        for part in data.chunks(size) {
            output.extend(parser.feed(part).unwrap());
        }
        parser.finish().unwrap();
        assert_eq!(output, vec![b"first".to_vec(), b"second".to_vec()]);
    }
}

#[test]
fn compressed_frames_and_trailer() {
    let mut parser = ConnectParser::new(100, true);
    assert_eq!(
        parser
            .feed(&[frame(&gzip(b"hello"), 1), frame(&gzip(b"{}"), 3)].concat())
            .unwrap(),
        vec![b"hello".to_vec()]
    );
    parser.finish().unwrap();
}

#[test]
fn error_trailers_and_truncation_fail() {
    for input in [
        frame(b"hello", 0),
        frame(b"[]", 2),
        frame(b"broken", 2),
        frame(br#"{"error":{"code":"internal","message":"secret"}}"#, 2),
        frame(br#"{"error":null}"#, 2),
        [frame(b"{}", 2), frame(b"late", 0)].concat(),
        vec![0, 0, 0],
        vec![4, 0, 0, 0, 0],
    ] {
        let mut parser = ConnectParser::new(100, false);
        let result = parser.feed(&input).and_then(|_| parser.finish());
        assert!(result.is_err());
        assert!(!result.unwrap_err().to_string().contains("secret"));
    }
}

#[test]
fn compression_size_and_trailing_bytes_fail() {
    let compressed = gzip(&vec![b'x'; 1000]);
    let mut trailing = gzip(b"hello");
    trailing.push(0);
    for (data, enabled) in [
        (frame(&compressed, 1), true),
        (frame(&trailing, 1), true),
        (frame(&gzip(b"hello"), 1), false),
        (frame(&[0; 101], 0), false),
    ] {
        assert!(ConnectParser::new(100, enabled).feed(&data).is_err());
    }
}
