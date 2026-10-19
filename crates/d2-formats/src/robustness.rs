//! Property tests: arbitrary bytes must never panic any wave-1 parser, and
//! structurally valid inputs must decode.

use crate::excel::{self, ExcelTableKind, ExcelVersion};
use crate::tbl::StringTable;
use crate::txt::TxtTable;
use proptest::prelude::*;

proptest! {
    #[test]
    fn txt_random_bytes_never_panic_and_rows_cover_headers(
        bytes in prop::collection::vec(any::<u8>(), 0..200)
    ) {
        let t = TxtTable::parse(&bytes).unwrap();
        for row in t.rows() {
            prop_assert!(row.cells().len() >= t.headers().len());
        }
    }

    #[test]
    fn tbl_random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = StringTable::parse(&bytes);
    }

    #[test]
    fn tbl_plausible_header_with_random_body_never_panics(
        h in 0u16..4,
        n in 0u32..6,
        body in prop::collection::vec(any::<u8>(), 0..200),
    ) {
        let mut bytes = vec![0u8; 0x15];
        bytes[2..4].copy_from_slice(&h.to_le_bytes());
        bytes[4..8].copy_from_slice(&n.to_le_bytes());
        bytes.extend_from_slice(&body);
        let _ = StringTable::parse(&bytes);
    }

    #[test]
    fn excel_random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        for kind in ExcelTableKind::all() {
            let _ = excel::decode_bin(*kind, ExcelVersion::V113c, &bytes);
            let _ = excel::detect_version(*kind, &bytes);
        }
        let _ = excel::decode_levels_bin(ExcelVersion::V114d, &bytes, &bytes);
    }

    #[test]
    fn excel_valid_count_with_random_records_decodes(
        records in 0usize..4,
        fill in prop::collection::vec(any::<u8>(), 1..64),
    ) {
        for kind in ExcelTableKind::all() {
            let size = excel::record_size(*kind, ExcelVersion::V113c);
            let mut bytes = (records as u32).to_le_bytes().to_vec();
            for i in 0..records * size {
                bytes.push(fill[i % fill.len()]);
            }
            let t = excel::decode_bin(*kind, ExcelVersion::V113c, &bytes).unwrap();
            prop_assert_eq!(t.len(), records);
            prop_assert_eq!(
                excel::detect_version(*kind, &bytes),
                Some(ExcelVersion::V113c)
            );
        }
    }
}
