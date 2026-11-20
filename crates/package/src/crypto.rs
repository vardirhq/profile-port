//! Authenticated encryption and key derivation.
//!
//! - Data: XChaCha20-Poly1305 with a random 192-bit nonce per message, so
//!   nonces never need to be tracked across interrupted sessions.
//! - Key wrapping: Argon2id(transfer code, salt) → key-encryption key.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::code::TransferCode;
use crate::error::{Error, Result};

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// Bytes added to every sealed message.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

pub(crate) struct Key(Zeroizing<[u8; 32]>);

impl Key {
    pub fn random() -> Self {
        Self(Zeroizing::new(random_bytes()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(Self(Zeroizing::new(arr)))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(&self.0[..]))
    }
}

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).expect("operating system RNG unavailable");
    out
}

/// Encrypt and authenticate `plaintext`, binding it to `aad`.
/// Output: nonce ‖ ciphertext ‖ tag.
pub(crate) fn seal(key: &Key, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let nonce: [u8; NONCE_LEN] = random_bytes();
    let ct = key
        .cipher()
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad })
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory buffers");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Inverse of [`seal`]. `None` if the data or `aad` were altered, or the key
/// is wrong.
pub(crate) fn open(key: &Key, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < OVERHEAD {
        return None;
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    key.cipher().decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad }).ok()
}

/// Argon2id parameters, stored in the package header so they can be raised
/// in future versions without breaking old packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: [u8; 16],
}

impl KdfParams {
    /// Production parameters: 64 MiB, 3 passes. Roughly 0.2–0.5 s on an
    /// older PC, which is what makes offline guessing of a 50-bit code
    /// impractical.
    pub fn recommended() -> Self {
        Self { m_cost_kib: 64 * 1024, t_cost: 3, p_cost: 1, salt: random_bytes() }
    }

    /// Minimal cost, for tests only. Never use for real packages.
    #[doc(hidden)]
    pub fn insecure_fast_for_tests() -> Self {
        Self { m_cost_kib: 8, t_cost: 1, p_cost: 1, salt: random_bytes() }
    }
}

pub(crate) fn derive_kek(code: &TransferCode, p: &KdfParams) -> Result<Key> {
    let params = argon2::Params::new(p.m_cost_kib, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| Error::Corrupt(format!("invalid key-derivation parameters: {e}")))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 32]);
    let secret = Zeroizing::new(code.secret());
    argon
        .hash_password_into(&secret[..], &p.salt, &mut out[..])
        .map_err(|e| Error::Corrupt(format!("key derivation failed: {e}")))?;
    Ok(Key(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_tamper_detection() {
        let key = Key::random();
        let sealed = seal(&key, b"aad", b"hello");
        assert_eq!(open(&key, b"aad", &sealed).unwrap(), b"hello");
        assert!(open(&key, b"other aad", &sealed).is_none());
        assert!(open(&Key::random(), b"aad", &sealed).is_none());
        let mut flipped = sealed.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(open(&key, b"aad", &flipped).is_none());
        assert!(open(&key, b"aad", &sealed[..10]).is_none());
    }

    #[test]
    fn kek_depends_on_code_and_salt() {
        let code = TransferCode::generate();
        let p = KdfParams::insecure_fast_for_tests();
        let a = derive_kek(&code, &p).unwrap();
        let b = derive_kek(&code, &p).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
        let c = derive_kek(&TransferCode::generate(), &p).unwrap();
        assert_ne!(a.as_bytes(), c.as_bytes());
        let d = derive_kek(&code, &KdfParams::insecure_fast_for_tests()).unwrap();
        assert_ne!(a.as_bytes(), d.as_bytes());
    }
}
