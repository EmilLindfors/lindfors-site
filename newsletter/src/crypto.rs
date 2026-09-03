//! Addresses at rest.
//!
//! Every address in the database is sealed with XChaCha20-Poly1305 under `DATA_KEY`,
//! which lives in the environment file and nowhere else. The lookup key for a row is
//! the HMAC pseudonym from `tokens::event_subject`, so the service can find, insert
//! and delete a subscriber without decrypting anything, and only decrypts when it is
//! about to send.
//!
//! What this buys: a copy of the database, or of a nightly dump, names nobody. What it
//! does not buy: protection from anyone who has both the dump and the environment
//! file, which is root on this box either way. The key is the thing to keep a second
//! copy of -- lose it and the list is unrecoverable, since it is the only place the
//! addresses exist.
//!
//! Blob layout: 24-byte random nonce, then ciphertext with the 16-byte tag. A fresh
//! nonce per seal, so two rows for the same address never compare equal.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

const NONCE_LEN: usize = 24;

pub struct Vault {
    cipher: XChaCha20Poly1305,
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not a hex string".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

impl Vault {
    /// From `DATA_KEY`: 32 bytes as 64 hex characters, `openssl rand -hex 32`.
    pub fn from_hex_key(hex: &str) -> Result<Self, String> {
        let bytes = from_hex(hex).map_err(|e| format!("DATA_KEY: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!("DATA_KEY must be 32 bytes (64 hex characters), got {}", bytes.len()));
        }
        Ok(Self {
            cipher: XChaCha20Poly1305::new(Key::from_slice(&bytes)),
        })
    }

    pub fn seal(&self, plaintext: &str) -> Vec<u8> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .expect("XChaCha20-Poly1305 encryption cannot fail on in-memory data");
        out.extend_from_slice(&ct);
        out
    }

    pub fn open(&self, blob: &[u8]) -> Result<String, String> {
        if blob.len() < NONCE_LEN + 16 {
            return Err("sealed value too short".into());
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let plain = self
            .cipher
            .decrypt(XNonce::from_slice(nonce), ct)
            .map_err(|_| "sealed value did not verify -- wrong DATA_KEY, or a tampered row".to_string())?;
        String::from_utf8(plain).map_err(|e| format!("sealed value is not UTF-8: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn a_sealed_address_opens_again() {
        let v = Vault::from_hex_key(KEY).unwrap();
        let blob = v.seal("someone@example.com");
        assert_eq!(v.open(&blob).unwrap(), "someone@example.com");
        assert_eq!(blob.len(), NONCE_LEN + "someone@example.com".len() + 16);
    }

    /// Two seals of one address differ, so the ciphertext is not itself a pseudonym.
    #[test]
    fn nonces_are_fresh() {
        let v = Vault::from_hex_key(KEY).unwrap();
        assert_ne!(v.seal("a@example.com"), v.seal("a@example.com"));
    }

    #[test]
    fn a_tampered_blob_or_wrong_key_is_refused() {
        let v = Vault::from_hex_key(KEY).unwrap();
        let mut blob = v.seal("a@example.com");
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(v.open(&blob).is_err());
        let other = Vault::from_hex_key(&KEY.replace('0', "f")).unwrap();
        assert!(other.open(&v.seal("a@example.com")).is_err());
        assert!(v.open(b"short").is_err());
    }

    #[test]
    fn the_key_must_be_32_bytes_of_hex() {
        assert!(Vault::from_hex_key("abc").is_err());
        assert!(Vault::from_hex_key(&"zz".repeat(32)).is_err());
        assert!(Vault::from_hex_key(&"ab".repeat(31)).is_err());
        assert!(Vault::from_hex_key(&format!(" {KEY}\n")).is_ok(), "whitespace from the env file is tolerated");
    }
}
