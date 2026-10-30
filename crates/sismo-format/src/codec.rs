use crate::DecodeError;

/// Append-only byte encoder for the sismo primitives: LEB128 varints,
/// zigzag signed varints, little-endian floats and length-prefixed strings.
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
        }
    }

    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.buf.push(value);
        self
    }

    pub fn uvar(&mut self, mut value: u64) -> &mut Self {
        while value >= 0x80 {
            // Truncation is the point: the low 7 bits go out with the continuation bit.
            #[allow(clippy::cast_possible_truncation)]
            self.buf.push((value as u8) | 0x80);
            value >>= 7;
        }
        #[allow(clippy::cast_possible_truncation)]
        self.buf.push(value as u8);
        self
    }

    pub fn ivar(&mut self, value: i64) -> &mut Self {
        #[allow(clippy::cast_sign_loss)]
        let zigzag = ((value << 1) ^ (value >> 63)) as u64;
        self.uvar(zigzag)
    }

    pub fn f64(&mut self, value: f64) -> &mut Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.u8(u8::from(value))
    }

    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.uvar(value.len() as u64);
        self.buf.extend_from_slice(value);
        self
    }

    pub fn str(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }

    pub fn raw(&mut self, value: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(value);
        self
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

/// Cursor over encoded bytes; every read is bounds-checked.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// # Errors
    /// [`DecodeError::Truncated`] when no byte is left.
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        let byte = *self.data.get(self.pos).ok_or(DecodeError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    /// # Errors
    /// [`DecodeError::Truncated`] or [`DecodeError::VarintOverflow`].
    pub fn uvar(&mut self) -> Result<u64, DecodeError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            if shift >= 64 {
                return Err(DecodeError::VarintOverflow);
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    /// # Errors
    /// Same as [`Decoder::uvar`].
    pub fn ivar(&mut self) -> Result<i64, DecodeError> {
        let raw = self.uvar()?;
        #[allow(clippy::cast_possible_wrap)]
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    /// # Errors
    /// [`DecodeError::Truncated`] or [`DecodeError::OutOfRange`] when the
    /// varint does not fit in `u32`.
    pub fn u32var(&mut self) -> Result<u32, DecodeError> {
        u32::try_from(self.uvar()?).map_err(|_| DecodeError::OutOfRange)
    }

    /// # Errors
    /// [`DecodeError::Truncated`].
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(f64::from_le_bytes(array))
    }

    /// # Errors
    /// [`DecodeError::Truncated`].
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }

    /// # Errors
    /// [`DecodeError::Truncated`] when fewer than `len` bytes remain.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// # Errors
    /// [`DecodeError::Truncated`] or [`DecodeError::OutOfRange`].
    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = usize::try_from(self.uvar()?).map_err(|_| DecodeError::OutOfRange)?;
        self.take(len)
    }

    /// # Errors
    /// [`DecodeError::InvalidUtf8`] besides the [`Decoder::bytes`] errors.
    pub fn str(&mut self) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| DecodeError::InvalidUtf8)
    }

    /// # Errors
    /// Same as [`Decoder::str`].
    pub fn string(&mut self) -> Result<String, DecodeError> {
        self.str().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        let values = [0u64, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX];
        let signed = [0i64, -1, 1, -64, 64, i64::MIN, i64::MAX];
        let mut enc = Encoder::default();
        for v in values {
            enc.uvar(v);
        }
        for v in signed {
            enc.ivar(v);
        }
        enc.f64(-2.5).str("héllo").bool(true);
        let bytes = enc.into_inner();
        let mut dec = Decoder::new(&bytes);
        for v in values {
            assert_eq!(dec.uvar().unwrap(), v);
        }
        for v in signed {
            assert_eq!(dec.ivar().unwrap(), v);
        }
        assert!((dec.f64().unwrap() + 2.5).abs() < f64::EPSILON);
        assert_eq!(dec.str().unwrap(), "héllo");
        assert!(dec.bool().unwrap());
        assert!(dec.is_empty());
    }

    #[test]
    fn truncated_input_errors() {
        let mut dec = Decoder::new(&[0x80]);
        assert_eq!(dec.uvar(), Err(DecodeError::Truncated));
        let mut dec = Decoder::new(&[5, b'a']);
        assert_eq!(dec.str(), Err(DecodeError::Truncated));
    }
}
