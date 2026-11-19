//! MD5 (RFC 1321), for the one place a specification asks for it: a
//! thumbnail's file is named by the MD5 of the address of what it shows.
//! It guards nothing here, so its weakness as a seal does not matter.
//! Written out because no crate for it is among gevrek's dependencies.

/// How far each step turns its sum to the left.
const TURNS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// The sum of `data`, as its sixteen bytes.
pub fn sum(data: &[u8]) -> [u8; 16] {
    // The constants are the sines of 1 to 64, as the RFC defines them.
    let mut sines = [0u32; 64];
    for (index, sine) in sines.iter_mut().enumerate() {
        *sine = (((index + 1) as f64).sin().abs() * 4_294_967_296.0) as u32;
    }
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    // The message, a one bit, zeros up to 56 of 64, and its length in bits.
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for step in 0..64 {
            let (mixed, word) = match step / 16 {
                0 => ((b & c) | (!b & d), step),
                1 => ((d & b) | (!d & c), (5 * step + 1) % 16),
                2 => (b ^ c ^ d, (3 * step + 5) % 16),
                _ => (c ^ (b | !d), (7 * step) % 16),
            };
            let sum = a
                .wrapping_add(mixed)
                .wrapping_add(sines[step])
                .wrapping_add(words[word]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(TURNS[step]));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *bytes = word.to_le_bytes();
    }
    out
}

/// The sum as it is written: 32 small hexadecimal digits.
pub fn hex(data: &[u8]) -> String {
    sum(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sums_of_the_rfc_come_out() {
        // The test suite of RFC 1321, appendix A.5.
        for (text, sum) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(hex(text.as_bytes()), sum, "{text:?}");
        }
        // Lengths around the edge of a block (55, 56, 63, 64, 65 bytes):
        // where the padding goes into a block of its own.
        for (length, sum) in [
            (55, "ef1772b6dff9a122358552954ad0df65"),
            (56, "3b0c8ac703f828b04c6c197006d17218"),
            (63, "b06521f39153d618550606be297466d5"),
            (64, "014842d480b571495a4a0363793f7367"),
            (65, "c743a45e0d2e6a95cb859adae0248435"),
        ] {
            assert_eq!(hex(&vec![b'a'; length]), sum, "{length} bytes");
        }
        // The example of the thumbnail specification.
        assert_eq!(
            hex(b"file:///home/jens/photos/me.png"),
            "c6ee772d9e49320e97ec29a7eb5b1697"
        );
    }
}
