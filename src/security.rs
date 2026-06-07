use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::AppState;

pub(crate) async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let is_admin = request.uri().path().starts_with("/admin");
    if is_admin
        && request.method() == axum::http::Method::POST
        && request.uri().path() != "/admin/login"
        && !has_same_origin(request.headers())
    {
        return (StatusCode::FORBIDDEN, "Cross-origin admin request rejected").into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    if is_admin {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

pub(crate) fn is_admin(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == "admin_session").then_some(value)
            })
        })
        == Some(state.session_token.as_str())
}

pub(crate) fn has_same_origin(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    if let Some(origin) = headers.get(header::ORIGIN) {
        return origin
            .to_str()
            .ok()
            .and_then(strip_http_scheme)
            .is_some_and(|origin_host| origin_host == host);
    }
    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(strip_http_scheme)
        .is_some_and(|referer| referer == host || referer.starts_with(&format!("{host}/")))
}

fn strip_http_scheme(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
}
