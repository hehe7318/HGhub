use crate::api::{self, Api};
use axum::body::Body;
use axum::extract::{Form, Path, RawQuery, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    api: Arc<Api>,
    password_sha256: Arc<[u8]>,
    session_token: Arc<str>,
    cookie_secure: bool,
    data_dir: Arc<PathBuf>,
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

fn login_html(error: bool) -> String {
    let error = if error {
        "<p class=\"error\">密码错误</p>"
    } else {
        ""
    };
    format!(
        r##"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#151515"><title>HGhub 登录</title>
<style>
*{{box-sizing:border-box}}html,body{{margin:0;min-height:100%;background:#151515;color:#f5f5f5;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
body{{display:grid;place-items:center;padding:24px}}main{{width:min(100%,360px);padding:28px;border:1px solid #333;border-radius:18px;background:#202020}}
h1{{margin:0 0 8px;font-size:24px}}p{{color:#aaa;margin:0 0 22px}}.error{{color:#ff7777;margin:-10px 0 14px}}
input,button{{width:100%;height:48px;border-radius:12px;font-size:16px}}input{{border:1px solid #444;background:#111;color:#fff;padding:0 14px;margin-bottom:14px}}
button{{border:0;background:#f2c94c;color:#171717;font-weight:700}}
</style></head><body><main><h1>HGhub</h1><p>请输入访问密码</p>{error}
<form method="post" action="/auth/login"><input type="password" name="password" required autofocus autocomplete="current-password">
<button type="submit">登录</button></form></main></body></html>"##
    )
}

async fn login_page() -> Response {
    let mut response = Html(login_html(false)).into_response();
    security_headers(&mut response);
    response
}

fn security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        "nosniff".parse().unwrap(),
    );
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        "no-referrer".parse().unwrap(),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        "default-src 'self' blob: data:; connect-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
            .parse()
            .unwrap(),
    );
}

fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}

fn authorized(headers: &axum::http::HeaderMap, state: &AppState) -> bool {
    cookie_value(headers, "hghub_session")
        .is_some_and(|value| value == state.session_token.as_ref())
}

async fn login(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let actual = Sha256::digest(form.password.as_bytes());
    if !bool::from(actual.as_slice().ct_eq(state.password_sha256.as_ref())) {
        let mut response = (StatusCode::UNAUTHORIZED, Html(login_html(true))).into_response();
        security_headers(&mut response);
        return response;
    }
    let mut response = Redirect::to("/").into_response();
    let secure = if state.cookie_secure { "; Secure" } else { "" };
    let cookie = format!(
        "hghub_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{secure}",
        state.session_token,
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie.parse().expect("valid session cookie"),
    );
    security_headers(&mut response);
    response
}

async fn logout() -> Response {
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        "hghub_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
            .parse()
            .expect("valid logout cookie"),
    );
    response
}

async fn require_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if authorized(request.headers(), &state) {
        let mut response = next.run(request).await;
        security_headers(&mut response);
        return response;
    }
    let mut response = if request.uri().path().starts_with("/api/")
        || request.uri().path().starts_with("/media/")
    {
        StatusCode::UNAUTHORIZED.into_response()
    } else {
        Redirect::to("/login").into_response()
    };
    security_headers(&mut response);
    response
}

async fn invoke(
    State(state): State<AppState>,
    Path(command): Path<String>,
    Json(args): Json<Value>,
) -> Response {
    // A web client must never turn the host into a proxy for its private network.
    // The desktop toggle is local-only; sharing the web app does not grant it.
    if command == "security_set" && args.get("strict").and_then(Value::as_bool) == Some(false) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "ok": false, "error": "网页版不允许关闭内网访问保护" })),
        )
            .into_response();
    }
    if command == "debug_log" {
        if let Some(message) = args.get("msg").and_then(Value::as_str) {
            let _ = crate::logging::append_debug(&state.data_dir, message);
        }
        return Json(json!({ "ok": true, "data": null })).into_response();
    }
    match api::web_dispatch(&state.api, &command, args).await {
        Ok(data) => Json(json!({ "ok": true, "data": data })).into_response(),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "ok": false, "error": error })),
        )
            .into_response(),
    }
}

async fn media(Path(kind): Path<String>, RawQuery(query): RawQuery) -> Response {
    let path = format!("/{kind}");
    let response = crate::media::handle(&path, query.as_deref().unwrap_or(""), "/media").await;
    let (parts, body) = response.into_parts();
    let mut output = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        output = output.header(name, value);
    }
    output
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("缺少环境变量 {name}"))
}

pub async fn run() -> Result<(), String> {
    let password_sha256 = hex::decode(required_env("HGHUB_WEB_PASSWORD_SHA256")?)
        .map_err(|_| "HGHUB_WEB_PASSWORD_SHA256 必须是 64 位十六进制 SHA-256".to_string())?;
    if password_sha256.len() != 32 {
        return Err("HGHUB_WEB_PASSWORD_SHA256 必须是 64 位十六进制 SHA-256".into());
    }
    let session_token = required_env("HGHUB_WEB_SESSION_TOKEN")?;
    if session_token.len() < 32
        || !session_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("HGHUB_WEB_SESSION_TOKEN 至少需要 32 个 URL 安全字符".into());
    }
    let cookie_secure = match std::env::var("HGHUB_WEB_COOKIE_SECURE")
        .unwrap_or_else(|_| "true".to_string())
        .as_str()
    {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => return Err("HGHUB_WEB_COOKIE_SECURE 只能为 true 或 false".into()),
    };
    let bind: SocketAddr = std::env::var("HGHUB_WEB_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8900".to_string())
        .parse()
        .map_err(|_| "HGHUB_WEB_BIND 格式错误".to_string())?;
    let dist =
        PathBuf::from(std::env::var("HGHUB_WEB_DIST").unwrap_or_else(|_| "dist".to_string()));
    if !dist.join("index.html").is_file() {
        return Err(format!("网页目录缺少 index.html: {}", dist.display()));
    }
    let data_dir = PathBuf::from(
        std::env::var("HGHUB_WEB_DATA").unwrap_or_else(|_| "/var/lib/hghub-web".to_string()),
    );
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    crate::net::bind_base(&data_dir);
    crate::network_policy::bind_security(&data_dir);
    crate::playback_log::bind(&data_dir);
    let api = Arc::new(Api::new());
    api.session.bind(data_dir.clone());

    let state = AppState {
        api,
        password_sha256: password_sha256.into(),
        session_token: session_token.into(),
        cookie_secure,
        data_dir: Arc::new(data_dir),
    };
    let static_files =
        ServeDir::new(&dist).not_found_service(ServeFile::new(dist.join("index.html")));
    let protected = Router::new()
        .route("/health", get(health))
        .route("/auth/logout", post(logout))
        .route("/api/invoke/{command}", post(invoke))
        .route("/media/{kind}", get(media))
        .fallback_service(static_files)
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));
    let app = Router::new()
        .route("/login", get(login_page))
        .route("/auth/login", post(login))
        .merge(protected)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("监听 {bind} 失败: {e}"))?;
    println!("HGhub Web listening on http://{bind}");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("HTTP 服务退出: {e}"))
}
