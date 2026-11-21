use anyhow::{anyhow, bail};
use base64::Engine;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::db::Db;

const ENGINE: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub async fn mount_key(db: &Db) -> anyhow::Result<[u8; 32]> {
    let secret = db.federation_mount_key().await?;
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0_u8; 32];
    key.copy_from_slice(&digest);
    Ok(key)
}

pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow!("invalid federation mount key"))?;
    let mut nonce = [0_u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| anyhow!("could not encrypt federation token"))?;
    let mut payload = nonce.to_vec();
    payload.extend_from_slice(&ciphertext);
    Ok(ENGINE.encode(payload))
}

pub fn decrypt(key: &[u8; 32], encoded: &str) -> anyhow::Result<String> {
    let payload = ENGINE
        .decode(encoded)
        .map_err(|_| anyhow!("invalid federation token ciphertext"))?;
    if payload.len() <= 24 {
        bail!("invalid federation token ciphertext");
    }
    let (nonce, ciphertext) = payload.split_at(24);
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow!("invalid federation mount key"))?;
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("could not decrypt federation token"))?;
    String::from_utf8(plaintext).map_err(|_| anyhow!("federation token is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::{decrypt, encrypt};

    #[test]
    fn round_trips() {
        let key = [7_u8; 32];
        let encoded = encrypt(&key, "fhfed_secret").unwrap();
        assert_eq!(decrypt(&key, &encoded).unwrap(), "fhfed_secret");
        let other = [8_u8; 32];
        assert!(decrypt(&other, &encoded).is_err());
    }
}
