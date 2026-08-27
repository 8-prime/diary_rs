use axum::{
    Form,
    extract::{FromRequestParts, State},
    http::request::Parts,
    response::Redirect,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use domain::auth::{Admin, SESSION_TTL};
use serde::Deserialize;

use crate::state::AppState;

pub const SESSION_COOKIE: &str = "session";

/// The whole admin check, in one place. Any handler that takes an `Admin`
/// argument gets this run before its body does, and a handler that does not
/// take one cannot obtain an `Admin` by other means: the type has a private
/// field, so `AuthService::validate` is the only thing that can build one.
///
/// This is why it is an extractor rather than a `tower` layer -- a layer would
/// have to smuggle the result through request extensions, and nothing would
/// stop a handler from forgetting to look.
impl FromRequestParts<AppState> for Admin {
    type Rejection = Redirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);

        let Some(cookie) = jar.get(SESSION_COOKIE) else {
            return Err(Redirect::to("/login"));
        };

        return state
            .auth
            .validate(cookie.value())
            .map_err(|_| Redirect::to("/login"));
    }
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub token: String,
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> (CookieJar, Redirect) {
    let Ok(session) = state.auth.login(&form.token) else {
        // Back to the form. There is no account to enumerate here, so the
        // only thing a message could leak is that the token was wrong.
        return (jar, Redirect::to("/login?failed=1"));
    };

    let cookie = Cookie::build((SESSION_COOKIE, session.token().to_string()))
        .path("/")
        // The cookie is the session: script has no reason to read it, and
        // Lax still lets a shared link land on the site normally.
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.secure_cookies)
        // Expire the cookie alongside the server-side session so the two
        // cannot disagree about whether you are logged in.
        .max_age(time::Duration::seconds(SESSION_TTL.as_secs() as i64))
        .build();

    return (jar.add(cookie), Redirect::to("/admin"));
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> (CookieJar, Redirect) {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        // A cookie we cannot parse is one we cannot have issued, and either
        // way the answer is the same: drop it.
        let _ = state.auth.logout(cookie.value());
    }

    // The removal has to carry the same path as the cookie it clears, or the
    // browser keeps the original.
    let removal = Cookie::build((SESSION_COOKIE, "")).path("/").build();

    return (jar.remove(removal), Redirect::to("/login"));
}
