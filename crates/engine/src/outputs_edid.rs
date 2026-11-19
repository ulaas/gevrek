//! What a display says of itself (its EDID), and the name gevrek knows it
//! by wherever it is plugged in.
//!
//! A display's device path (Windows) or connector (Linux) says where it is
//! plugged in: the same display on another port is another path. Its EDID
//! says what it is: who made it, which model, and (when the maker wrote one)
//! its serial number. The backends read the raw block; this reads it and
//! makes the id of it, the same on both systems.
//!
//! The id (`MonitorInfo::stable_id`):
//!
//! - `GSM-5B7F-0004E2A1`: maker, product code, the serial number of the
//!   block's head (all in the block's own writing: three letters, four and
//!   eight hexadecimal digits).
//! - `GSM-5B7F-0004E2A1-304NTAB1C234`: the same with the serial number the
//!   display also gives as text. Both are in the id when both are there:
//!   some makers write the same number into every display and tell them
//!   apart by the text alone, others the other way round.
//! - `GSM-5B7F-noserial`: the display gives no serial number (the number is
//!   zero and there is no text). Two displays of one model without serial
//!   numbers have the same EDID and cannot be told apart by it; see
//!   `number_same_stable_ids`, which tells them apart by where they are
//!   plugged in.
//! - `port:HDMI-A-1`, `port:\\?\DISPLAY#...`: no EDID could be read (a
//!   nested or headless output, a display made up by a program, a driver
//!   that gives none). The id is then the place and stays with the port.
//!
//! What the id cannot do: a display that gives another EDID on each of its
//! inputs (some give another product code over HDMI than over DisplayPort)
//! is another display to everyone who reads it, and so here.

/// The eight bytes every EDID begins with.
const HEAD: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
/// How long the base block is. What follows it (extension blocks) says
/// nothing of who the display is.
pub const BLOCK: usize = 128;
/// Where the four descriptors of the base block begin, and how long each is.
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];
const DESCRIPTOR: usize = 18;
/// A descriptor that holds the display's name, and one that holds its
/// serial number as text.
const NAME: u8 = 0xFC;
const SERIAL_TEXT: u8 = 0xFF;

/// Who a display is, as the base block of its EDID says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// The maker's three letters (`GSM`), as the PnP registry gives them out.
    pub maker: String,
    /// The maker's number for the model.
    pub product: u16,
    /// The serial number of the block's head; 0 when the maker wrote none.
    pub serial: u32,
    /// The week it was made in (1 to 54), 0 when not said, and the year.
    /// (A week of 255 says the year is the model's, not the making's.)
    pub week: u8,
    pub year: u16,
    /// The name the display gives itself (`LG ULTRAGEAR`), if it gives one.
    pub name: Option<String>,
    /// The serial number it gives as text, if it gives one.
    pub serial_text: Option<String>,
}

/// The three letters of a maker out of the two bytes they are packed into
/// (five bits a letter, 1 for `A`; the first of the two bytes holds the
/// high bits). `None` when the bytes hold something that is no letter.
pub fn maker_of(first: u8, second: u8) -> Option<String> {
    let packed = u16::from_be_bytes([first, second]);
    if packed & 0x8000 != 0 {
        return None;
    }
    [packed >> 10, packed >> 5, packed]
        .into_iter()
        .map(|bits| match (bits & 0x1F) as u8 {
            letter @ 1..=26 => Some(char::from(b'A' + letter - 1)),
            _ => None,
        })
        .collect()
}

/// The text of a descriptor: up to the line feed that ends it, without the
/// spaces that fill it. Only what can be read everywhere is kept (the text
/// is in an old code page; what is beyond ASCII is left out).
fn text_of(field: &[u8]) -> Option<String> {
    let text: String = field
        .iter()
        .take_while(|byte| **byte != 0x0A && **byte != 0x00)
        .filter(|byte| byte.is_ascii_graphic() || **byte == b' ')
        .map(|byte| char::from(*byte))
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

impl Edid {
    /// Read the base block out of `bytes` (the block alone, or with the
    /// extension blocks after it). `None` for anything that is no EDID:
    /// too short, another head, a sum that is not right, a maker that is no
    /// three letters.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let block = bytes.get(..BLOCK)?;
        if block[..8] != HEAD {
            return None;
        }
        // All bytes of a block add up to a multiple of 256.
        if block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return None;
        }
        let maker = maker_of(block[8], block[9])?;
        let text = |wanted: u8| {
            DESCRIPTORS
                .iter()
                .map(|at| &block[*at..*at + DESCRIPTOR])
                // (A descriptor that begins with a number is a mode's
                // timing, not a text.)
                .filter(|part| part[..3] == [0, 0, 0] && part[3] == wanted)
                .find_map(|part| text_of(&part[5..]))
        };
        Some(Self {
            maker,
            product: u16::from_le_bytes([block[10], block[11]]),
            serial: u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
            week: block[16],
            year: 1990 + u16::from(block[17]),
            name: text(NAME),
            serial_text: text(SERIAL_TEXT),
        })
    }

    /// Whether the display says which one of its model it is.
    pub fn has_serial(&self) -> bool {
        self.serial != 0 || self.serial_text.is_some()
    }

    /// The id of this display wherever it is plugged in (see the module).
    pub fn stable_id(&self) -> String {
        let mut id = stable_id_of_model(&self.maker, self.product);
        if self.serial != 0 {
            id.push_str(&format!("-{:08X}", self.serial));
        }
        if let Some(text) = &self.serial_text {
            id.push('-');
            // (No spaces and no signs in an id: it is written into files
            // and given on command lines.)
            id.extend(text.chars().map(|letter| {
                if letter.is_ascii_alphanumeric() || matches!(letter, '-' | '_' | '.') {
                    letter
                } else {
                    '_'
                }
            }));
        }
        if !self.has_serial() {
            id.push_str(NO_SERIAL);
        }
        id
    }
}

/// What an id ends with when the display gives no serial number.
const NO_SERIAL: &str = "-noserial";

fn stable_id_of_model(maker: &str, product: u16) -> String {
    format!("{maker}-{product:04X}")
}

/// The id of a display of which only the maker and the model are known
/// (Windows tells those two of a display whose EDID it does not hand out):
/// that of a display without a serial number. `maker` is the two bytes as
/// they are in the EDID.
pub fn stable_id_without_serial(maker: [u8; 2], product: u16) -> Option<String> {
    let maker = maker_of(maker[0], maker[1])?;
    Some(format!(
        "{}{NO_SERIAL}",
        stable_id_of_model(&maker, product)
    ))
}

/// The id of a display that has no EDID: the place it is at (its connector,
/// its device path, the name of a nested or headless output). It stays with
/// the port, not with the display.
pub fn stable_id_of_place(place: &str) -> String {
    format!("port:{}", place.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor that holds a text.
    fn text_descriptor(kind: u8, text: &str) -> [u8; DESCRIPTOR] {
        let mut part = [0x20u8; DESCRIPTOR];
        part[..5].copy_from_slice(&[0, 0, 0, kind, 0]);
        let bytes = text.as_bytes();
        part[5..5 + bytes.len()].copy_from_slice(bytes);
        if bytes.len() < 13 {
            part[5 + bytes.len()] = 0x0A;
        }
        part
    }

    /// A base block as a display gives it: version 1.4, a mode's timing in
    /// the first descriptor, the texts after it, the sum made right.
    fn block(maker: [u8; 2], product: u16, serial: u32, texts: &[(u8, &str)]) -> Vec<u8> {
        let mut block = vec![0u8; BLOCK];
        block[..8].copy_from_slice(&HEAD);
        block[8..10].copy_from_slice(&maker);
        block[10..12].copy_from_slice(&product.to_le_bytes());
        block[12..16].copy_from_slice(&serial.to_le_bytes());
        (block[16], block[17]) = (23, 32);
        (block[18], block[19]) = (1, 4);
        // 2560x1440: a timing begins with its pixel clock, which is not 0.
        block[54..58].copy_from_slice(&[0x56, 0x5E, 0x00, 0xA0]);
        for (at, (kind, text)) in DESCRIPTORS[1..].iter().zip(texts) {
            block[*at..*at + DESCRIPTOR].copy_from_slice(&text_descriptor(*kind, text));
        }
        let sum = block[..127]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        block[127] = 0u8.wrapping_sub(sum);
        block
    }

    /// `GSM`, as LG's displays have it.
    const GSM: [u8; 2] = [0x1E, 0x6D];

    #[test]
    fn the_makers_letters_are_read_out_of_their_two_bytes() {
        assert_eq!(maker_of(0x1E, 0x6D).as_deref(), Some("GSM"));
        assert_eq!(maker_of(0x10, 0xAC).as_deref(), Some("DEL"));
        assert_eq!(maker_of(0x06, 0x10).as_deref(), Some("APP"));
        // Nothing at all, a letter beyond Z, the bit that is never set.
        assert_eq!(maker_of(0x00, 0x00), None);
        assert_eq!(maker_of(0x7F, 0xFF), None);
        assert_eq!(maker_of(0x9E, 0x6D), None);
    }

    #[test]
    fn a_display_is_read_out_of_its_block_and_given_its_id() {
        let cases: [(&str, Vec<u8>, Option<&str>); 9] = [
            (
                "a serial number in the head and one as text",
                block(
                    GSM,
                    0x5B7F,
                    0x0004_E2A1,
                    &[(NAME, "LG ULTRAGEAR"), (SERIAL_TEXT, "304NTAB1C234")],
                ),
                Some("GSM-5B7F-0004E2A1-304NTAB1C234"),
            ),
            (
                "a serial number in the head alone",
                block(GSM, 0x5C4D, 16_843_009, &[(NAME, "LG HDR 4K")]),
                Some("GSM-5C4D-01010101"),
            ),
            (
                "a serial number as text alone, with what an id does not hold",
                block([0x10, 0xAC], 0xA0C4, 0, &[(SERIAL_TEXT, "7M/K 12#A")]),
                Some("DEL-A0C4-7M_K_12_A"),
            ),
            (
                "no serial number",
                block(GSM, 0x5B7F, 0, &[(NAME, "LG ULTRAGEAR")]),
                Some("GSM-5B7F-noserial"),
            ),
            (
                "a text that fills its descriptor has no line feed",
                block(GSM, 0x0001, 0, &[(SERIAL_TEXT, "ABCDEFGHIJKLM")]),
                Some("GSM-0001-ABCDEFGHIJKLM"),
            ),
            (
                "the extension blocks after it are not looked at",
                [block(GSM, 0x5B7F, 7, &[]), vec![0xAB; BLOCK]].concat(),
                Some("GSM-5B7F-00000007"),
            ),
            (
                "a sum that is not right",
                {
                    let mut bad = block(GSM, 0x5B7F, 7, &[]);
                    bad[12] ^= 0x40;
                    bad
                },
                None,
            ),
            (
                "another head",
                {
                    // (The sum is still right: the two bytes are swapped.)
                    let mut bad = block(GSM, 0x5B7F, 7, &[]);
                    bad.swap(0, 1);
                    bad
                },
                None,
            ),
            (
                "a maker that is no three letters",
                block([0x00, 0x00], 0x5B7F, 7, &[]),
                None,
            ),
        ];
        for (what, bytes, id) in cases {
            let read = Edid::parse(&bytes);
            assert_eq!(read.map(|edid| edid.stable_id()).as_deref(), id, "{what}");
        }
    }

    #[test]
    fn what_is_too_short_or_no_edid_is_none() {
        let whole = block(GSM, 0x5B7F, 7, &[(NAME, "LG ULTRAGEAR")]);
        assert!(Edid::parse(&whole).is_some());
        for length in [0, 1, 8, 20, 127] {
            assert_eq!(Edid::parse(&whole[..length]), None, "{length} bytes");
        }
        assert_eq!(Edid::parse(&[0u8; BLOCK]), None);
        assert_eq!(Edid::parse(&[0xFFu8; 4 * BLOCK]), None);
        assert_eq!(Edid::parse(b"no edid at all"), None);
    }

    #[test]
    fn everything_the_head_and_the_texts_say_is_read() {
        let read = Edid::parse(&block(
            GSM,
            0x5B7F,
            0x0004_E2A1,
            &[(SERIAL_TEXT, "304NTAB1C234"), (NAME, "LG ULTRAGEAR")],
        ))
        .expect("an EDID");
        assert_eq!(
            read,
            Edid {
                maker: "GSM".to_string(),
                product: 0x5B7F,
                serial: 0x0004_E2A1,
                week: 23,
                year: 2022,
                name: Some("LG ULTRAGEAR".to_string()),
                serial_text: Some("304NTAB1C234".to_string()),
            }
        );
        assert!(read.has_serial());

        // A text of spaces alone is none; another kind of text (0xFE, a
        // remark) is neither name nor serial number.
        let bare = Edid::parse(&block(
            GSM,
            0x5B7F,
            0,
            &[(SERIAL_TEXT, "   "), (0xFE, "a remark")],
        ))
        .expect("an EDID");
        assert_eq!((bare.name, bare.serial_text), (None, None));
    }

    #[test]
    fn a_display_without_an_edid_has_an_id_all_the_same() {
        // What Windows tells of a display whose EDID it does not hand out.
        assert_eq!(
            stable_id_without_serial(GSM, 0x5C4D).as_deref(),
            Some("GSM-5C4D-noserial")
        );
        assert_eq!(stable_id_without_serial([0, 0], 0x5C4D), None);
        // The same as the block of that display gives without a serial.
        assert_eq!(
            Edid::parse(&block(GSM, 0x5C4D, 0, &[])).map(|edid| edid.stable_id()),
            stable_id_without_serial(GSM, 0x5C4D)
        );
        assert_eq!(stable_id_of_place(" HDMI-A-1 "), "port:HDMI-A-1");
        assert_eq!(stable_id_of_place("winit"), "port:winit");
    }
}
