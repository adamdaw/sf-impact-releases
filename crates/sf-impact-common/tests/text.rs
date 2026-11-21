//! The shared text contract, including encodings that previously disagreed.
use sf_impact_common::text;

#[test]
fn utf8_keeps_its_optional_bom() {
    assert_eq!(
        text(b"\xef\xbb\xbfCircle").as_deref(),
        Some("\u{feff}Circle")
    );
    assert_eq!(text(b"").as_deref(), Some(""));
}

#[test]
fn latin1_is_not_text() {
    let bytes: Vec<u8> = (0x80..=0xff).collect();
    assert_eq!(text(&bytes), None);
}

#[test]
fn utf16_requires_a_bom_and_decodes_without_error() {
    assert_eq!(text(&[0xff, 0xfe, 0xe9, 0x00]).as_deref(), Some("é"));
    assert_eq!(text(&[0xfe, 0xff, 0x00, 0xe9]).as_deref(), Some("é"));
    assert_eq!(text(&[0xe9, 0x00]), None);
    assert_eq!(text(&[0x00, 0xe9]), None);
    assert_eq!(text(&[0xff, 0xfe, 0x00, 0xd8]), None);
    assert_eq!(text(&[0xfe, 0xff, 0xd8, 0x00]), None);
    assert_eq!(text(&[0xff, 0xfe, 0x43]), None);
    assert_eq!(text(&[0xfe, 0xff, 0x43]), None);
}

#[test]
fn nul_in_utf8_is_not_text() {
    assert_eq!(text(b"Circle\0\n"), None);
    assert_eq!(text(b"C\0"), None);
    assert_eq!(text(b"\0C"), None);
    let mut late_nul = vec![b'a'; 8001];
    late_nul[8000] = 0;
    assert_eq!(text(&late_nul), None);
}

#[test]
fn nul_in_utf16_is_not_text() {
    assert_eq!(text(&[0xff, 0xfe, 0x43, 0, 0, 0]), None);
    assert_eq!(text(&[0xfe, 0xff, 0, 0x43, 0, 0]), None);
}

#[test]
fn utf32_le_with_a_bom_is_not_text() {
    assert_eq!(text(&[0xff, 0xfe, 0, 0, 0x43, 0, 0, 0]), None);
}

#[test]
fn mangled_png_that_is_valid_utf8_is_not_text() {
    assert_eq!(text(b"\xef\xbf\xbdPNG\r\n\x1a\n\0\0\0\rIHDR"), None);
}
