//! Transfer codes: the short secret that unlocks one package.
//!
//! Format (see ADR 0002): 10 random symbols from Crockford base32 (50 bits)
//! followed by 2 check symbols, shown as `XXXX-XXXX-XXXX`. Entry is
//! case-insensitive, ignores hyphens and spaces, and maps the look-alikes
//! O→0 and I/L→1 so they can never be mistyped.

use std::fmt;
use zeroize::Zeroize;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const DATA_SYMBOLS: usize = 10;
const CHECK_SYMBOLS: usize = 2;
pub const CODE_SYMBOLS: usize = DATA_SYMBOLS + CHECK_SYMBOLS;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    #[error("a transfer code has {CODE_SYMBOLS} characters (like 7KQ2-M9XD-4PWH), this one has {0}")]
    WrongLength(usize),
    #[error("'{0}' is not used in transfer codes")]
    InvalidCharacter(char),
    #[error("that code isn't valid; check it against the ticket")]
    ChecksumMismatch,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TransferCode {
    /// Symbol values, each 0..32.
    symbols: [u8; DATA_SYMBOLS],
}

impl TransferCode {
    /// A new random code from the operating system's CSPRNG.
    pub fn generate() -> Self {
        let mut symbols = [0u8; DATA_SYMBOLS];
        getrandom::getrandom(&mut symbols).expect("operating system RNG unavailable");
        // 256 is a multiple of 32, so masking keeps the distribution uniform.
        for s in &mut symbols {
            *s &= 31;
        }
        Self { symbols }
    }

    pub fn parse(input: &str) -> Result<Self, CodeError> {
        let mut values = Vec::with_capacity(CODE_SYMBOLS);
        for c in input.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            values.push(decode_symbol(c).ok_or(CodeError::InvalidCharacter(c))?);
        }
        if values.len() != CODE_SYMBOLS {
            return Err(CodeError::WrongLength(values.len()));
        }
        let mut symbols = [0u8; DATA_SYMBOLS];
        symbols.copy_from_slice(&values[..DATA_SYMBOLS]);
        let ok = check_symbols(&symbols) == [values[DATA_SYMBOLS], values[DATA_SYMBOLS + 1]];
        values.zeroize();
        if !ok {
            symbols.zeroize();
            return Err(CodeError::ChecksumMismatch);
        }
        Ok(Self { symbols })
    }

    /// Canonical secret bytes fed to the key-derivation function.
    pub(crate) fn secret(&self) -> [u8; DATA_SYMBOLS] {
        self.symbols.map(|s| ALPHABET[s as usize])
    }
}

fn check_symbols(symbols: &[u8; DATA_SYMBOLS]) -> [u8; CHECK_SYMBOLS] {
    let hash = blake3::Hasher::new().update(b"profileport transfer code check v1").update(symbols).finalize();
    let b = hash.as_bytes();
    [b[0] & 31, b[1] & 31]
}

fn decode_symbol(c: char) -> Option<u8> {
    let c = match c.to_ascii_uppercase() {
        'O' => '0',
        'I' | 'L' => '1',
        c => c,
    };
    ALPHABET.iter().position(|&a| a as char == c).map(|p| p as u8)
}

impl fmt::Display for TransferCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let check = check_symbols(&self.symbols);
        let all = self.symbols.iter().chain(check.iter());
        for (i, s) in all.enumerate() {
            if i > 0 && i % 4 == 0 {
                f.write_str("-")?;
            }
            write!(f, "{}", ALPHABET[*s as usize] as char)?;
        }
        Ok(())
    }
}

/// Never print the code by accident (logs, panics).
impl fmt::Debug for TransferCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TransferCode(<redacted>)")
    }
}

impl Drop for TransferCode {
    fn drop(&mut self) {
        self.symbols.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_display() {
        for _ in 0..200 {
            let code = TransferCode::generate();
            let shown = code.to_string();
            assert_eq!(shown.len(), 14);
            assert_eq!(&shown[4..5], "-");
            assert_eq!(&shown[9..10], "-");
            assert_eq!(TransferCode::parse(&shown).unwrap(), code);
        }
    }

    #[test]
    fn entry_is_forgiving() {
        let code = TransferCode::generate();
        let shown = code.to_string();
        let messy = format!("  {} ", shown.to_lowercase().replace('-', " "));
        assert_eq!(TransferCode::parse(&messy).unwrap(), code);
        // Look-alikes decode to the same symbol.
        let with_lookalikes = shown.replace('0', "O").replace('1', "l");
        assert_eq!(TransferCode::parse(&with_lookalikes).unwrap(), code);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(TransferCode::parse("ABC").unwrap_err(), CodeError::WrongLength(3));
        assert_eq!(TransferCode::parse("7KQ2-M9XD-4PWU").unwrap_err(), CodeError::InvalidCharacter('U'));
    }

    #[test]
    fn check_symbols_catch_almost_all_single_typos() {
        let mut caught = 0;
        let mut total = 0;
        for _ in 0..100 {
            let shown: Vec<char> = TransferCode::generate().to_string().replace('-', "").chars().collect();
            for pos in 0..shown.len() {
                for &sub in ALPHABET.iter() {
                    let sub = sub as char;
                    if sub == shown[pos] {
                        continue;
                    }
                    let mut typo = shown.clone();
                    typo[pos] = sub;
                    total += 1;
                    if TransferCode::parse(&typo.iter().collect::<String>()).is_err() {
                        caught += 1;
                    }
                }
            }
        }
        // 10 check bits: expect ~99.9 % detection.
        assert!(caught as f64 / total as f64 > 0.995, "{caught}/{total}");
    }

    #[test]
    fn debug_is_redacted() {
        let code = TransferCode::generate();
        assert!(!format!("{code:?}").contains(&code.to_string()[..4]));
    }
}
