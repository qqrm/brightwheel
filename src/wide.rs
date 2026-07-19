use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub(crate) fn encode_null_terminated(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn decode_null_terminated(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

#[cfg(test)]
mod tests {
    use super::{decode_null_terminated, encode_null_terminated};

    #[test]
    fn encodes_a_single_terminal_null() {
        assert_eq!(
            encode_null_terminated("BrightWheel"),
            [66, 114, 105, 103, 104, 116, 87, 104, 101, 101, 108, 0]
        );
    }

    #[test]
    fn decoding_stops_at_the_first_null() {
        assert_eq!(decode_null_terminated(&[66, 114, 105, 0, 88]), "Bri");
    }

    #[test]
    fn decoding_accepts_an_unterminated_slice() {
        assert_eq!(decode_null_terminated(&[72, 68, 82]), "HDR");
    }
}
