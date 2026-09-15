use axum_extra::extract::cookie::{Cookie, SameSite};

pub fn build_cookie<'a, C: Into<Cookie<'a>>>(c: C, max_age: Option<u64>) -> Cookie<'a> {
    let mut builder = Cookie::build(c)
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::None);

    if let Some(max_age) = max_age {
        builder = builder.max_age(time::Duration::seconds(max_age.cast_signed()));
    }

    builder.build()
}
