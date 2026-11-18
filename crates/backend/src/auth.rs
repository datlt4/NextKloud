//! Auth primitives: argon2 hashing + HS256 JWT. Pure logic, fully unit-tested.
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid, // user id
    pub adm: bool,
    pub exp: usize,
}

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }
    hash_password_unchecked(password)
}

/// Bypass policy — chỉ dùng khi seed admin mặc định (must_reset bắt đổi ngay).
pub fn hash_password_unchecked(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

pub fn create_jwt(user: Uuid, admin: bool, enc: &EncodingKey) -> Result<String, String> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    encode(&Header::default(), &Claims { sub: user, adm: admin, exp }, enc)
        .map_err(|e| e.to_string())
}

pub fn verify_jwt(token: &str, dec: &DecodingKey) -> Result<Claims, String> {
    decode::<Claims>(token, dec, &Validation::default())
        .map(|d| d.claims)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_and_wrong_rejected() {
        let h = hash_password("correct-horse-1").unwrap();
        assert!(verify_password("correct-horse-1", &h));
        assert!(!verify_password("wrong-pass-2", &h));
        assert!(!verify_password("correct-horse-1", "not-a-hash"));
    }

    #[test]
    fn short_password_rejected() {
        assert!(hash_password("short1").is_err());
    }

    #[test]
    fn jwt_roundtrip() {
        let enc = EncodingKey::from_secret(b"test-secret-that-is-long-enough");
        let dec = DecodingKey::from_secret(b"test-secret-that-is-long-enough");
        let id = Uuid::new_v4();
        let tok = create_jwt(id, true, &enc).unwrap();
        let c = verify_jwt(&tok, &dec).unwrap();
        assert_eq!(c.sub, id);
        assert!(c.adm);
    }

    #[test]
    fn jwt_wrong_secret_rejected() {
        let enc = EncodingKey::from_secret(b"secret-a-long-enough-1");
        let dec = DecodingKey::from_secret(b"secret-b-long-enough-1");
        let tok = create_jwt(Uuid::new_v4(), false, &enc).unwrap();
        assert!(verify_jwt(&tok, &dec).is_err());
        assert!(verify_jwt("garbage.token.here", &dec).is_err());
    }

    #[test]
    fn jwt_expired_rejected() {
        let enc = EncodingKey::from_secret(b"secret-long-enough-x");
        let dec = DecodingKey::from_secret(b"secret-long-enough-x");
        let past = (chrono::Utc::now() - chrono::Duration::days(1)).timestamp() as usize;
        let tok = encode(
            &Header::default(),
            &Claims { sub: Uuid::new_v4(), adm: false, exp: past },
            &enc,
        )
        .unwrap();
        assert!(verify_jwt(&tok, &dec).is_err());
    }
}
