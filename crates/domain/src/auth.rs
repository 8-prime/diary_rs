use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use rand::Rng;
use subtle::ConstantTimeEq;

use crate::{Error, Result};

pub struct Admin(());

// 7 Days
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 7);

pub struct AuthService {
    sessions: std::sync::RwLock<HashMap<[u8; 32], Instant>>, // admin token from env
    admin_token: String, //admin cookie value can genuenly just be a map of logged in cookie values and session structs
}

pub struct Session {
    token: String,
}

impl Session {
    /// The value to put in the cookie.
    pub fn token(&self) -> &str {
        return &self.token;
    }
}

/// Hand-written so a stray `{:?}` in a log line cannot print a live session
/// token, which is as good as a password.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        return f.debug_struct("Session").field("token", &"<redacted>").finish();
    }
}

impl AuthService {
    pub fn new(admin_token: &str) -> Self {
        AuthService {
            sessions: std::sync::RwLock::new(HashMap::new()),
            admin_token: admin_token.to_string(),
        }
    }

    // must return result of the valid admin cookie value to be used
    pub fn login(&self, admin_token: &str) -> Result<Session> {
        if !bool::from(self.admin_token.as_bytes().ct_eq(admin_token.as_bytes())) {
            return Err(Error::Unauthorized);
        }

        let expiry = Instant::now() + SESSION_TTL;
        let mut raw_token = [0u8; 32];
        rand::rng().fill_bytes(&mut raw_token);
        let token = URL_SAFE.encode(raw_token);

        let mut w = self.sessions.write().expect("sessions lock poisoned");
        _ = w.insert(raw_token, expiry.clone());

        return Ok(Session { token: token });
    }

    pub fn logout(&self, session_token: &str) -> Result<()> {
        let decoded = URL_SAFE
            .decode(session_token)
            .map_err(|_| Error::BadToken)?;
        let raw_token: [u8; 32] = decoded.try_into().map_err(|_| Error::BadToken)?;

        let mut w = self.sessions.write().expect("sessions lock poisoned");
        w.remove(&raw_token);

        return Ok(());
    }

    pub fn validate(&self, session_token: &str) -> Result<Admin> {
        let decoded = URL_SAFE
            .decode(session_token)
            .map_err(|_| Error::BadToken)?;
        let raw_token: [u8; 32] = decoded.try_into().map_err(|_| Error::BadToken)?;

        let mut w = self.sessions.write().expect("sessions lock poisoned");

        let Some(expiry) = w.get(&raw_token).copied() else {
            return Err(Error::Unauthorized);
        };

        if expiry > Instant::now() {
            return Ok(Admin(()));
        }

        w.remove(&raw_token);
        return Err(Error::Unauthorized);
    }
}
