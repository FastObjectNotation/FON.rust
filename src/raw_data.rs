use crate::error::FonError;


// Z85 alphabet: 85 printable ASCII characters.
const Z85_ENCODE: &[u8; 85] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-:+=^!/*?&<>()[]{}@%$#";


// Z85 decode table maps ASCII 32-127 to 0-84, 255 = invalid.
const Z85_DECODE: [u8; 96] = [
    255, 68, 255, 84, 83, 82, 72, 255, 75, 76, 70, 65, 255, 63, 62, 69, 0, 1, 2, 3, 4, 5, 6, 7, 8,
    9, 64, 255, 73, 66, 74, 71, 81, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 77, 255, 78, 67, 255, 255, 10, 11, 12, 13, 14, 15, 16,
    17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 79, 255, 80, 255,
    255,
];


pub struct RawData {
    data: Vec<u8>,
    encoded: String,
}


impl RawData {
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            encoded: String::new(),
        }
    }


    pub fn from_encoded(encoded: String) -> Self {
        Self {
            data: Vec::new(),
            encoded,
        }
    }


    pub fn data(&self) -> &[u8] {
        &self.data
    }


    pub fn encoded(&self) -> &str {
        &self.encoded
    }


    pub fn is_packed(&self) -> bool {
        !self.encoded.is_empty()
    }


    pub fn is_unpacked(&self) -> bool {
        !self.data.is_empty()
    }


    pub fn pack(&mut self) -> &mut Self {
        if !self.encoded.is_empty() || self.data.is_empty() {
            return self;
        }

        let input_len = self.data.len();
        let padding = (4 - (input_len % 4)) % 4;
        let padded_len = input_len + padding;
        let mut output_len = (padded_len / 4) * 5;

        if padding > 0 {
            output_len += 1;
        }

        let mut out = vec![0u8; output_len];
        let mut write_pos = 0;

        let full_blocks = input_len / 4;
        for i in 0..full_blocks {
            let offset = i * 4;
            let mut value: u32 = (u32::from(self.data[offset]) << 24)
                | (u32::from(self.data[offset + 1]) << 16)
                | (u32::from(self.data[offset + 2]) << 8)
                | u32::from(self.data[offset + 3]);

            out[write_pos + 4] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 3] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 2] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 1] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos] = Z85_ENCODE[value as usize];
            write_pos += 5;
        }

        let remaining = input_len % 4;
        if remaining > 0 {
            let mut value: u32 = 0;
            let offset = full_blocks * 4;

            for i in 0..remaining {
                value = (value << 8) | u32::from(self.data[offset + i]);
            }
            for _ in remaining..4 {
                value <<= 8;
            }

            out[write_pos + 4] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 3] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 2] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos + 1] = Z85_ENCODE[(value % 85) as usize];
            value /= 85;
            out[write_pos] = Z85_ENCODE[value as usize];
            write_pos += 5;

            out[write_pos] = b'0' + padding as u8;
        }

        // All bytes in `out` are members of Z85_ENCODE, which is pure ASCII.
        self.encoded = String::from_utf8(out).expect("Z85 alphabet is ASCII");
        self.data.clear();
        self
    }


    pub fn unpack(&mut self) -> Result<&mut Self, FonError> {
        if !self.data.is_empty() || self.encoded.is_empty() {
            return Ok(self);
        }

        let bytes = self.encoded.as_bytes();
        let mut len = bytes.len();
        if len == 0 {
            return Ok(self);
        }

        let mut padding: usize = 0;
        let last = bytes[len - 1];
        if (b'1'..=b'3').contains(&last) {
            padding = (last - b'0') as usize;
            len -= 1;
        }

        let output_len = (len / 5) * 4 - padding;
        let mut out = vec![0u8; output_len];
        let mut write_pos = 0;

        let mut i = 0;
        while i < len {
            let mut value: u32 = 0;
            for j in 0..5 {
                let c = bytes[i + j];
                if !(32..=127).contains(&c) {
                    return Err(FonError::Parse("Invalid Z85 character".into()));
                }
                let decoded = Z85_DECODE[(c - 32) as usize];
                if decoded == 255 {
                    return Err(FonError::Parse("Invalid Z85 character".into()));
                }
                value = value.wrapping_mul(85).wrapping_add(decoded as u32);
            }

            if write_pos < output_len {
                out[write_pos] = ((value >> 24) & 0xFF) as u8;
                write_pos += 1;
            }
            if write_pos < output_len {
                out[write_pos] = ((value >> 16) & 0xFF) as u8;
                write_pos += 1;
            }
            if write_pos < output_len {
                out[write_pos] = ((value >> 8) & 0xFF) as u8;
                write_pos += 1;
            }
            if write_pos < output_len {
                out[write_pos] = (value & 0xFF) as u8;
                write_pos += 1;
            }

            i += 5;
        }

        self.data = out;
        self.encoded.clear();
        Ok(self)
    }
}
