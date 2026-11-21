//! Self-hosted ALTCHA proof-of-work captcha (v1 challenge format, which the v3 widget accepts).
//!
//! Challenge: the server picks a secret `number`, publishes `challenge = sha256(salt + number)`
//! and `signature = hmac(key, challenge)`. The browser brute-forces `number`; the server checks
//! the hash, the signature, the expiry embedded in the salt, and that the solution is unused.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const ALGORITHM: &str = "SHA-256";

pub struct Altcha {
    key: Vec<u8>,
    /// Upper bound for the secret number; ~average solve cost is half this many hashes.
    max_number: u64,
    ttl: Duration,
    /// Solved challenges -> expiry (unix secs), so each solution works once.
    used: Mutex<HashMap<String, u64>>,
}

#[derive(Serialize)]
pub struct Challenge {
    algorithm: &'static str,
    challenge: String,
    maxnumber: u64,
    salt: String,
    signature: String,
}

#[derive(Deserialize)]
struct Payload {
    algorithm: String,
    challenge: String,
    number: u64,
    salt: String,
    signature: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sha256_hex(salt: &str, number: u64) -> String {
    hex::encode(Sha256::digest(format!("{salt}{number}")))
}

/// Reads `expires=<unix secs>` from the salt's query part (`<random>?expires=...`).
fn salt_expiry(salt: &str) -> Option<u64> {
    let (_, query) = salt.split_once('?')?;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("expires=")?.parse().ok())
}

impl Altcha {
    pub fn new(key: Vec<u8>, max_number: u64, ttl: Duration) -> Self {
        Self {
            key,
            max_number,
            ttl,
            used: Mutex::default(),
        }
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length")
    }

    pub fn create_challenge(&self) -> Challenge {
        let mut rng = rand::rng();
        let nonce: [u8; 12] = rng.random();
        let salt = format!(
            "{}?expires={}",
            hex::encode(nonce),
            now_secs() + self.ttl.as_secs()
        );
        let number = rng.random_range(0..=self.max_number);
        let challenge = sha256_hex(&salt, number);
        let mut mac = self.mac();
        mac.update(challenge.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        Challenge {
            algorithm: ALGORITHM,
            challenge,
            maxnumber: self.max_number,
            salt,
            signature,
        }
    }

    /// Verifies a base64 payload from the widget and marks it used.
    pub fn verify(&self, payload_b64: &str) -> Result<(), &'static str> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(payload_b64.trim())
            .map_err(|_| "payload is not base64")?;
        let p: Payload = serde_json::from_slice(&raw).map_err(|_| "payload is malformed")?;

        if p.algorithm != ALGORITHM {
            return Err("unsupported algorithm");
        }
        let now = now_secs();
        let expires = salt_expiry(&p.salt).ok_or("salt has no expiry")?;
        if expires <= now {
            return Err("challenge expired");
        }
        if sha256_hex(&p.salt, p.number) != p.challenge.to_ascii_lowercase() {
            return Err("wrong solution");
        }
        // Signature proves we issued this challenge (and so its salt/expiry).
        let sig = hex::decode(&p.signature).map_err(|_| "bad signature")?;
        let mut mac = self.mac();
        mac.update(p.challenge.to_ascii_lowercase().as_bytes());
        mac.verify_slice(&sig).map_err(|_| "bad signature")?;

        let mut used = self.used.lock().unwrap();
        used.retain(|_, &mut exp| exp > now);
        if used
            .insert(p.challenge.to_ascii_lowercase(), expires)
            .is_some()
        {
            return Err("challenge already used");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solve(c: &Challenge) -> String {
        let number = (0..=c.maxnumber)
            .find(|&n| sha256_hex(&c.salt, n) == c.challenge)
            .unwrap();
        let json = serde_json::json!({
            "algorithm": c.algorithm, "challenge": c.challenge, "number": number,
            "salt": c.salt, "signature": c.signature, "took": 1,
        });
        base64::engine::general_purpose::STANDARD.encode(json.to_string())
    }

    fn altcha() -> Altcha {
        Altcha::new(b"test-key".to_vec(), 1000, Duration::from_secs(60))
    }

    #[test]
    fn solves_once() {
        let a = altcha();
        let payload = solve(&a.create_challenge());
        assert_eq!(a.verify(&payload), Ok(()));
        assert_eq!(a.verify(&payload), Err("challenge already used"));
    }

    #[test]
    fn rejects_foreign_key_and_garbage() {
        let payload = solve(&altcha().create_challenge());
        let other = Altcha::new(b"other".to_vec(), 1000, Duration::from_secs(60));
        assert_eq!(other.verify(&payload), Err("bad signature"));
        assert!(other.verify("not base64!").is_err());
        let test_mode = base64::engine::general_purpose::STANDARD
            .encode(r#"{"challenge":null,"solution":null,"test":true}"#);
        assert!(other.verify(&test_mode).is_err());
    }

    #[test]
    fn rejects_expired_and_tampered_salt() {
        let a = Altcha::new(b"k".to_vec(), 1000, Duration::ZERO);
        assert_eq!(
            a.verify(&solve(&a.create_challenge())),
            Err("challenge expired")
        );

        // Extending the expiry changes the hash, so the original solution no longer matches.
        let a = altcha();
        let mut c = a.create_challenge();
        let n = (0..=c.maxnumber)
            .find(|&n| sha256_hex(&c.salt, n) == c.challenge)
            .unwrap();
        c.salt = format!(
            "{}?expires={}",
            c.salt.split('?').next().unwrap(),
            now_secs() + 999_999
        );
        let json = serde_json::json!({
            "algorithm": c.algorithm, "challenge": c.challenge, "number": n,
            "salt": c.salt, "signature": c.signature,
        });
        let payload = base64::engine::general_purpose::STANDARD.encode(json.to_string());
        assert_eq!(a.verify(&payload), Err("wrong solution"));
    }
}
