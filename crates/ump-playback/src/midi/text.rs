//! Decoding of MIDI meta text fields.

/// Decode a meta text field into a `String`.
///
/// Standard MIDI files carry no encoding marker, and the specification says
/// ASCII, which almost nobody honours. UTF-8 is tried first because it
/// validates itself; Shift-JIS is the fallback, being what Japanese sequencers
/// have written for decades. Anything else is decoded as Shift-JIS too, which
/// degrades to Latin-1-ish text rather than dropping the field.
pub fn decode_meta_text(bytes: &[u8]) -> String {
    let trimmed = trim_ascii(bytes);
    if trimmed.is_empty() {
        return String::new();
    }
    match std::str::from_utf8(trimmed) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(trimmed);
            decoded.into_owned()
        }
    }
}

/// Trim ASCII whitespace and stray NULs, which show up as padding in files
/// written by older sequencers.
fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let is_padding = |b: u8| b.is_ascii_whitespace() || b == 0;
    let start = bytes.iter().position(|&b| !is_padding(b));
    let Some(start) = start else { return &[] };
    let end = bytes.iter().rposition(|&b| !is_padding(b)).unwrap_or(start);
    &bytes[start..=end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ascii() {
        assert_eq!(decode_meta_text(b"Piano"), "Piano");
    }

    #[test]
    fn trims_padding_and_nuls() {
        assert_eq!(decode_meta_text(b"  Strings \0\0"), "Strings");
    }

    #[test]
    fn decodes_utf8() {
        assert_eq!(decode_meta_text("ピアノ".as_bytes()), "ピアノ");
    }

    #[test]
    fn falls_back_to_shift_jis() {
        // "ピアノ" in Shift-JIS, which is not valid UTF-8.
        let shift_jis = [0x83, 0x73, 0x83, 0x41, 0x83, 0x6D];
        assert_eq!(decode_meta_text(&shift_jis), "ピアノ");
    }

    #[test]
    fn empty_input_yields_empty_string() {
        assert_eq!(decode_meta_text(b""), "");
        assert_eq!(decode_meta_text(b"\0\0"), "");
    }
}
