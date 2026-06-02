use std::{
    collections::VecDeque,
    env,
    fmt::Write as _,
    path::{Path as FilePath, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Form, Multipart, Path, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
};
use pulldown_cmark::{Event, Options, Parser, html};
use rand::{Rng, distr::Alphanumeric};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::{fs, net::TcpListener};
use tower_http::services::ServeDir;
use uuid::Uuid;

const DEFAULT_HOME: &str = r#"# Hello, I'm George.

This is a small corner of the web for my work and notes.

[See my projects](/projects)
"#;
const ASSET_VERSION: &str = "20260602-1";

struct AppState {
    db: Mutex<Connection>,
    admin_password: String,
    session_token: String,
    uploads_dir: PathBuf,
    secure_cookie: bool,
    site_url: String,
    login_failures: Mutex<VecDeque<Instant>>,
}

#[derive(Clone)]
struct Project {
    id: i64,
    slug: String,
    title: String,
    summary: String,
    body: String,
    image_path: String,
    published: bool,
}

struct FooterLink {
    id: i64,
    label: String,
    url: String,
}

struct OwnedImage {
    id: i64,
    file_name: String,
    original_name: String,
}

#[derive(Debug)]
struct AppError(String);

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        eprintln!("request failed: {}", self.0);
        internal_server_error()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = PathBuf::from(env::var("DATA_DIR").unwrap_or_else(|_| "data".into()));
    let uploads_dir = data_dir.join("uploads");
    fs::create_dir_all(&uploads_dir).await?;

    let db = Connection::open(data_dir.join("site.db"))?;
    initialize_database(&db)?;
    cleanup_orphaned_uploads(&db, &uploads_dir).await?;

    let admin_password =
        env::var("ADMIN_PASSWORD").expect("ADMIN_PASSWORD must be set before starting the site");
    let state = Arc::new(AppState {
        db: Mutex::new(db),
        admin_password,
        session_token: random_token(),
        uploads_dir: uploads_dir.clone(),
        secure_cookie: env::var("COOKIE_SECURE").is_ok_and(|value| value == "true"),
        site_url: env::var("SITE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".into())
            .trim_end_matches('/')
            .to_string(),
        login_failures: Mutex::new(VecDeque::new()),
    });

    let app = Router::new()
        .route("/", get(home))
        .route("/projects", get(project_list))
        .route("/projects/{slug}", get(project_detail))
        .route("/healthz", get(healthz))
        .route("/robots.txt", get(robots_txt))
        .route("/sitemap.xml", get(sitemap_xml))
        .route("/admin/login", get(login_page).post(login))
        .route("/admin/logout", post(logout))
        .route("/admin", get(admin_dashboard))
        .route("/admin/settings", get(admin_settings).post(update_settings))
        .route("/admin/links", post(create_footer_link))
        .route("/admin/links/{id}/delete", post(delete_footer_link))
        .route("/admin/home", get(admin_home).post(update_home))
        .route("/admin/projects/new", get(new_project).post(create_project))
        .route(
            "/admin/projects/{id}/edit",
            get(edit_project).post(update_project),
        )
        .route("/admin/projects/{id}/delete", post(delete_project))
        .route("/admin/home/images", post(upload_home_image))
        .route("/admin/projects/{id}/images", post(upload_project_image))
        .route("/admin/images/{id}/delete", post(delete_image))
        .nest_service("/assets", ServeDir::new("static"))
        .nest_service("/uploads", ServeDir::new(uploads_dir))
        .fallback(fallback_not_found)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(middleware::from_fn(security_headers));
    #[cfg(debug_assertions)]
    let app = app.route("/__test/500", get(test_internal_server_error));
    let app = app.with_state(state);

    let address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    println!("Listening on http://{address}");
    let listener = TcpListener::bind(&address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn initialize_database(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA busy_timeout = 5000;

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            image_path TEXT NOT NULL DEFAULT '',
            published INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS footer_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            url TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL UNIQUE,
            original_name TEXT NOT NULL,
            owner_type TEXT NOT NULL CHECK (owner_type IN ('home', 'project')),
            owner_id INTEGER,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;
    db.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('home_markdown', ?1)",
        [DEFAULT_HOME],
    )?;
    db.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('copyright_claim', ?1)",
        ["© 2026 George. All rights reserved."],
    )?;
    for (key, value) in [
        ("site_title", "George"),
        (
            "home_seo_title",
            "George | Personal Website and Project Archive",
        ),
        (
            "site_description",
            "Personal website and project archive for George.",
        ),
        ("author_name", "George"),
        ("social_image", ""),
    ] {
        db.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    Ok(())
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
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
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self'; font-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    if is_admin {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

async fn cleanup_orphaned_uploads(db: &Connection, uploads_dir: &FilePath) -> Result<(), AppError> {
    let orphaned = {
        let mut statement = db.prepare(
            "SELECT file_name FROM images WHERE owner_type = 'project' AND owner_id NOT IN (SELECT id FROM projects)",
        )?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for file_name in orphaned {
        remove_upload_file(uploads_dir, &file_name).await?;
    }
    db.execute(
        "DELETE FROM images WHERE owner_type = 'project' AND owner_id NOT IN (SELECT id FROM projects)",
        [],
    )?;

    let tracked = {
        let mut statement = db.prepare("SELECT file_name FROM images")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut entries = fs::read_dir(uploads_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !tracked
            .iter()
            .any(|tracked_name| tracked_name == &file_name)
        {
            remove_upload_file(uploads_dir, &file_name).await?;
        }
    }
    Ok(())
}

async fn remove_upload_file(uploads_dir: &FilePath, file_name: &str) -> Result<(), AppError> {
    match fs::remove_file(uploads_dir.join(file_name)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn home(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let markdown = state.db.lock().unwrap().query_row(
        "SELECT value FROM settings WHERE key = 'home_markdown'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    Ok(Html(site_layout(
        &state,
        &setting(&state, "home_seo_title")?,
        &setting(&state, "site_description")?,
        "/",
        None,
        &markdown_to_html(&markdown),
        false,
    )?))
}

async fn project_list(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let projects = list_projects(&state, true)?;
    let mut content = String::from(
        "<header class=\"page-header\"><p class=\"eyebrow\">Archive</p><h1>Projects</h1><p>Things I have built, explored, and learned from.</p></header><div class=\"project-grid\">",
    );
    for project in projects {
        write!(
            content,
            "<article class=\"project-card\">{}<div><h2><a href=\"/projects/{}\">{}</a></h2><p>{}</p><a class=\"text-link\" href=\"/projects/{}\">Read more <span aria-hidden=\"true\">&rarr;</span></a></div></article>",
            project_image(&project),
            escape_html(&project.slug),
            escape_html(&project.title),
            escape_html(&project.summary),
            escape_html(&project.slug),
        )
        .unwrap();
    }
    content.push_str("</div>");
    Ok(Html(site_layout(
        &state,
        "Projects",
        "Projects, experiments, and selected work.",
        "/projects",
        None,
        &content,
        false,
    )?))
}

async fn project_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let project = find_project_by_slug(&state, &slug)?;
    let Some(project) = project.filter(|project| project.published) else {
        return Ok(not_found());
    };
    let content = format!(
        "<article class=\"project-detail\"><a class=\"text-link\" href=\"/projects\">&larr; All projects</a><header><p class=\"eyebrow\">Project</p><h1>{}</h1><p class=\"lede\">{}</p></header>{}{}</article>",
        escape_html(&project.title),
        escape_html(&project.summary),
        project_image(&project),
        markdown_to_html(&project.body),
    );
    Ok(Html(site_layout(
        &state,
        &project.title,
        &project.summary,
        &format!("/projects/{}", project.slug),
        (!project.image_path.is_empty()).then_some(project.image_path.as_str()),
        &content,
        false,
    )?)
    .into_response())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn robots_txt(State(state): State<Arc<AppState>>) -> String {
    format!(
        "User-agent: *\nAllow: /\nDisallow: /admin\nSitemap: {}/sitemap.xml\n",
        state.site_url
    )
}

async fn sitemap_xml(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let mut urls = format!(
        "<url><loc>{}/</loc></url><url><loc>{}/projects</loc></url>",
        escape_html(&state.site_url),
        escape_html(&state.site_url),
    );
    for project in list_projects(&state, true)? {
        write!(
            urls,
            "<url><loc>{}/projects/{}</loc></url>",
            escape_html(&state.site_url),
            escape_html(&project.slug),
        )
        .unwrap();
    }
    Ok((
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{urls}</urlset>"#),
    )
        .into_response())
}

async fn fallback_not_found() -> Response {
    not_found()
}

#[cfg(debug_assertions)]
async fn test_internal_server_error() -> Response {
    AppError("intentional debug error".into()).into_response()
}

async fn login_page() -> Html<String> {
    Html(layout(
        "Admin login",
        r#"<section class="admin-narrow"><p class="eyebrow">Private area</p><h1>Admin login</h1><form method="post" class="panel form-stack"><label>Password<input type="password" name="password" autocomplete="current-password" required></label><button type="submit">Log in</button></form></section>"#,
        false,
    ))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login(State(state): State<Arc<AppState>>, Form(form): Form<LoginForm>) -> Response {
    {
        let mut failures = state.login_failures.lock().unwrap();
        let cutoff = Instant::now() - Duration::from_secs(60);
        while failures.front().is_some_and(|failure| *failure < cutoff) {
            failures.pop_front();
        }
        if failures.len() >= 5 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many login attempts. Try again in one minute.",
            )
                .into_response();
        }
    }
    if form.password != state.admin_password {
        state
            .login_failures
            .lock()
            .unwrap()
            .push_back(Instant::now());
        return (
            StatusCode::UNAUTHORIZED,
            Html(layout(
                "Admin login",
                r#"<section class="admin-narrow"><h1>Admin login</h1><p class="error">Incorrect password.</p><a class="text-link" href="/admin/login">Try again</a></section>"#,
                false,
            )),
        )
            .into_response();
    }
    state.login_failures.lock().unwrap().clear();

    let secure = if state.secure_cookie { "; Secure" } else { "" };
    let cookie = format!(
        "admin_session={}; Path=/; HttpOnly; SameSite=Strict{}",
        state.session_token, secure
    );
    let mut response = Redirect::to("/admin").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    response
}

async fn logout() -> Response {
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("admin_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    response
}

async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let projects = list_projects(&state, false)?;
    let mut rows = String::new();
    for project in projects {
        write!(
            rows,
            "<tr><td><a href=\"/admin/projects/{}/edit\">{}</a></td><td>{}</td><td class=\"actions\"><a class=\"button secondary\" href=\"/admin/projects/{}/edit\">Edit</a><form method=\"post\" action=\"/admin/projects/{}/delete\"><button class=\"danger\" type=\"submit\">Delete</button></form></td></tr>",
            project.id,
            escape_html(&project.title),
            if project.published { "Published" } else { "Draft" },
            project.id,
            project.id,
        )
        .unwrap();
    }
    let content = format!(
        r#"<section class="admin-shell"><div class="admin-heading"><div><p class="eyebrow">Private area</p><h1>Site admin</h1></div><form method="post" action="/admin/logout"><button class="secondary" type="submit">Log out</button></form></div><nav class="admin-nav"><a href="/admin/home">Edit homepage</a><a href="/admin/projects/new">New project</a><a href="/admin/settings">Site settings</a></nav><div class="panel"><h2>Projects</h2><div class="table-wrap"><table><thead><tr><th>Title</th><th>Status</th><th></th></tr></thead><tbody>{rows}</tbody></table></div></div></section>"#
    );
    Ok(Html(layout("Admin", &content, true)).into_response())
}

async fn admin_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let copyright = setting(&state, "copyright_claim")?;
    let site_title = setting(&state, "site_title")?;
    let home_seo_title = setting(&state, "home_seo_title")?;
    let site_description = setting(&state, "site_description")?;
    let author_name = setting(&state, "author_name")?;
    let social_image = setting(&state, "social_image")?;
    let mut links_html = String::new();
    for link in list_footer_links(&state)? {
        write!(
            links_html,
            r#"<li><a href="{url}">{label}</a><form method="post" action="/admin/links/{id}/delete"><button class="danger" type="submit">Delete</button></form></li>"#,
            id = link.id,
            label = escape_html(&link.label),
            url = escape_html(&link.url),
        )
        .unwrap();
    }
    let content = format!(
        r#"<section class="admin-shell"><a class="text-link" href="/admin">&larr; Admin</a><h1>Site settings</h1><form method="post" class="panel form-stack"><label>Site title<input name="site_title" value="{}" required></label><label>Homepage SEO title <span class="hint">(used as the complete browser and search-result title)</span><input name="home_seo_title" value="{}" required></label><label>Author name<input name="author_name" value="{}" required></label><label>Search description<textarea name="site_description" rows="3" required>{}</textarea></label><label>Social image URL <span class="hint">(optional; used when sharing the site)</span><input name="social_image" value="{}"></label><label>Copyright claim<input name="copyright_claim" value="{}" required></label><button type="submit">Save settings</button></form><div class="panel settings-panel"><h2>Footer links</h2><ul class="admin-link-list">{links_html}</ul><form method="post" action="/admin/links" class="inline-form"><label>Label<input name="label" placeholder="GitHub" required></label><label>URL<input name="url" type="url" placeholder="https://github.com/..." required></label><button type="submit">Add link</button></form></div></section>"#,
        escape_html(&site_title),
        escape_html(&home_seo_title),
        escape_html(&author_name),
        escape_html(&site_description),
        escape_html(&social_image),
        escape_html(&copyright),
    );
    Ok(Html(layout("Footer settings", &content, true)).into_response())
}

#[derive(Deserialize)]
struct SettingsForm {
    site_title: String,
    home_seo_title: String,
    author_name: String,
    site_description: String,
    social_image: String,
    copyright_claim: String,
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    if !form.social_image.is_empty()
        && !is_http_url(&form.social_image)
        && !form.social_image.starts_with("/uploads/")
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            "Social image must be an http URL, an https URL, or an uploaded image path",
        )
            .into_response());
    }
    let db = state.db.lock().unwrap();
    for (key, value) in [
        ("site_title", form.site_title),
        ("home_seo_title", form.home_seo_title),
        ("author_name", form.author_name),
        ("site_description", form.site_description),
        ("social_image", form.social_image),
        ("copyright_claim", form.copyright_claim),
    ] {
        db.execute(
            "UPDATE settings SET value = ?1 WHERE key = ?2",
            params![value, key],
        )?;
    }
    Ok(Redirect::to("/admin/settings").into_response())
}

#[derive(Deserialize)]
struct FooterLinkForm {
    label: String,
    url: String,
}

async fn create_footer_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<FooterLinkForm>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    if !is_http_url(&form.url) {
        return Ok((
            StatusCode::BAD_REQUEST,
            "Link URL must start with http:// or https://",
        )
            .into_response());
    }
    state.db.lock().unwrap().execute(
        "INSERT INTO footer_links (label, url, sort_order) VALUES (?1, ?2, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM footer_links))",
        params![form.label, form.url],
    )?;
    Ok(Redirect::to("/admin/settings").into_response())
}

async fn delete_footer_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    state
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM footer_links WHERE id = ?1", [id])?;
    Ok(Redirect::to("/admin/settings").into_response())
}

async fn admin_home(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let markdown = state.db.lock().unwrap().query_row(
        "SELECT value FROM settings WHERE key = 'home_markdown'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let images = list_images(&state, "home", None)?;
    let content = format!(
        r#"<section class="admin-shell"><a class="text-link" href="/admin">&larr; Admin</a><h1>Edit homepage</h1><form method="post" class="panel form-stack"><label>Homepage Markdown<textarea class="markdown-editor" data-image-upload="/admin/home/images" name="markdown" rows="20" required>{}</textarea></label><button type="submit">Save homepage</button></form>{}</section>"#,
        escape_html(&markdown),
        image_library(&images),
    );
    Ok(Html(layout("Edit homepage", &content, true)).into_response())
}

#[derive(Deserialize)]
struct HomeForm {
    markdown: String,
}

async fn update_home(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<HomeForm>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    state.db.lock().unwrap().execute(
        "UPDATE settings SET value = ?1 WHERE key = 'home_markdown'",
        [form.markdown],
    )?;
    Ok(Redirect::to("/admin").into_response())
}

async fn new_project(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_admin(&headers, &state) {
        return Redirect::to("/admin/login").into_response();
    }
    Html(layout(
        "New project",
        &project_form("New project", "/admin/projects/new", None, &[]),
        true,
    ))
    .into_response()
}

#[derive(Deserialize)]
struct ProjectForm {
    title: String,
    slug: String,
    summary: String,
    body: String,
    image_path: String,
    published: Option<String>,
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let slug = normalized_slug(&form.slug, &form.title);
    if slug.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Project slug cannot be empty").into_response());
    }
    if state.db.lock().unwrap().execute(
        "INSERT INTO projects (slug, title, summary, body, image_path, published) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![slug, form.title, form.summary, form.body, form.image_path, form.published.is_some()],
    ).is_err() {
        return Ok((StatusCode::BAD_REQUEST, "Project slug is already in use").into_response());
    }
    Ok(Redirect::to("/admin").into_response())
}

async fn edit_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let Some(project) = find_project_by_id(&state, id)? else {
        return Ok(not_found());
    };
    let images = list_images(&state, "project", Some(id))?;
    Ok(Html(layout(
        "Edit project",
        &project_form(
            "Edit project",
            &format!("/admin/projects/{id}/edit"),
            Some(&project),
            &images,
        ),
        true,
    ))
    .into_response())
}

async fn update_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let slug = normalized_slug(&form.slug, &form.title);
    if slug.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Project slug cannot be empty").into_response());
    }
    if state.db.lock().unwrap().execute(
        "UPDATE projects SET slug = ?1, title = ?2, summary = ?3, body = ?4, image_path = ?5, published = ?6, updated_at = CURRENT_TIMESTAMP WHERE id = ?7",
        params![slug, form.title, form.summary, form.body, form.image_path, form.published.is_some(), id],
    ).is_err() {
        return Ok((StatusCode::BAD_REQUEST, "Project slug is already in use").into_response());
    }
    Ok(Redirect::to("/admin").into_response())
}

async fn delete_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let images = list_images(&state, "project", Some(id))?;
    for image in &images {
        remove_upload_file(&state.uploads_dir, &image.file_name).await?;
    }
    let db = state.db.lock().unwrap();
    db.execute(
        "DELETE FROM images WHERE owner_type = 'project' AND owner_id = ?1",
        [id],
    )?;
    db.execute("DELETE FROM projects WHERE id = ?1", [id])?;
    Ok(Redirect::to("/admin").into_response())
}

async fn upload_home_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if !is_admin(&headers, &state) {
        return Redirect::to("/admin/login").into_response();
    }
    store_image(&state, multipart, "home", None).await
}

async fn upload_project_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Response {
    if !is_admin(&headers, &state) {
        return Redirect::to("/admin/login").into_response();
    }
    match find_project_by_id(&state, id) {
        Ok(Some(_)) => store_image(&state, multipart, "project", Some(id)).await,
        Ok(None) => (StatusCode::NOT_FOUND, "Project not found").into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Serialize)]
struct ImageUploadData {
    #[serde(rename = "filePath")]
    file_path: String,
}

#[derive(Serialize)]
struct ImageUploadResponse {
    data: ImageUploadData,
}

async fn store_image(
    state: &AppState,
    mut multipart: Multipart,
    owner_type: &str,
    owner_id: Option<i64>,
) -> Response {
    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError(error.to_string()))
        .unwrap_or(None)
    else {
        return (StatusCode::BAD_REQUEST, Json(upload_error("noFileGiven"))).into_response();
    };
    let file_name = field.file_name().unwrap_or("upload");
    let original_name = file_name.to_string();
    let Some(extension) = allowed_image_extension(file_name) else {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(upload_error("typeNotAllowed")),
        )
            .into_response();
    };
    let stored_name = format!("{}.{}", Uuid::new_v4(), extension);
    let bytes = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, Json(upload_error("importError"))).into_response();
        }
    };
    if bytes.len() > 10 * 1024 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(upload_error("fileTooLarge")),
        )
            .into_response();
    }
    if !image_bytes_match_extension(extension, &bytes) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(upload_error("typeNotAllowed")),
        )
            .into_response();
    }
    if fs::write(state.uploads_dir.join(&stored_name), bytes)
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(upload_error("importError")),
        )
            .into_response();
    }
    if state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO images (file_name, original_name, owner_type, owner_id) VALUES (?1, ?2, ?3, ?4)",
            params![stored_name, original_name, owner_type, owner_id],
        )
        .is_err()
    {
        let _ = remove_upload_file(&state.uploads_dir, &stored_name).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(upload_error("importError")),
        )
            .into_response();
    }
    let image_path = format!("/uploads/{stored_name}");
    Json(ImageUploadResponse {
        data: ImageUploadData {
            file_path: image_path,
        },
    })
    .into_response()
}

fn upload_error(error: &str) -> serde_json::Value {
    serde_json::json!({ "error": error })
}

async fn delete_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    let file_name = state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT file_name FROM images WHERE id = ?1", [id], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    if let Some(file_name) = file_name {
        remove_upload_file(&state.uploads_dir, &file_name).await?;
        state
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM images WHERE id = ?1", [id])?;
    }
    Ok(Redirect::to("/admin").into_response())
}

fn list_projects(state: &AppState, only_published: bool) -> Result<Vec<Project>, AppError> {
    let db = state.db.lock().unwrap();
    let query = if only_published {
        "SELECT id, slug, title, summary, body, image_path, published FROM projects WHERE published = 1 ORDER BY created_at DESC"
    } else {
        "SELECT id, slug, title, summary, body, image_path, published FROM projects ORDER BY created_at DESC"
    };
    let mut statement = db.prepare(query)?;
    let projects = statement
        .query_map([], project_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(projects)
}

fn find_project_by_slug(state: &AppState, slug: &str) -> Result<Option<Project>, AppError> {
    let project = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id, slug, title, summary, body, image_path, published FROM projects WHERE slug = ?1",
            [slug],
            project_from_row,
        )
        .optional()?;
    Ok(project)
}

fn find_project_by_id(state: &AppState, id: i64) -> Result<Option<Project>, AppError> {
    let project = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id, slug, title, summary, body, image_path, published FROM projects WHERE id = ?1",
            [id],
            project_from_row,
        )
        .optional()?;
    Ok(project)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        summary: row.get(3)?,
        body: row.get(4)?,
        image_path: row.get(5)?,
        published: row.get(6)?,
    })
}

fn list_images(
    state: &AppState,
    owner_type: &str,
    owner_id: Option<i64>,
) -> Result<Vec<OwnedImage>, AppError> {
    let db = state.db.lock().unwrap();
    let mut statement = db.prepare(
        "SELECT id, file_name, original_name FROM images WHERE owner_type = ?1 AND owner_id IS ?2 ORDER BY created_at DESC",
    )?;
    let images = statement
        .query_map(params![owner_type, owner_id], |row| {
            Ok(OwnedImage {
                id: row.get(0)?,
                file_name: row.get(1)?,
                original_name: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(images)
}

fn image_library(images: &[OwnedImage]) -> String {
    let mut cards = String::new();
    for image in images {
        let path = format!("/uploads/{}", image.file_name);
        write!(
            cards,
            r#"<article class="image-card"><img src="{path}" alt=""><div><strong>{name}</strong><code>{path}</code><button class="secondary copy-image" type="button" data-copy="{path}">Copy URL</button><form method="post" action="/admin/images/{id}/delete"><button class="danger" type="submit">Delete</button></form></div></article>"#,
            id = image.id,
            name = escape_html(&image.original_name),
            path = escape_html(&path),
        )
        .unwrap();
    }
    format!(
        r#"<section class="image-library"><div><h2>Page images</h2><p>Paste, drop, or use the image button in the editor to upload. Copy a URL here for the project cover image.</p></div><div class="image-grid">{cards}</div></section>"#
    )
}

fn project_form(
    title: &str,
    action: &str,
    project: Option<&Project>,
    images: &[OwnedImage],
) -> String {
    let empty = Project {
        id: 0,
        slug: String::new(),
        title: String::new(),
        summary: String::new(),
        body: String::new(),
        image_path: String::new(),
        published: false,
    };
    let project = project.unwrap_or(&empty);
    let image_upload = if project.id == 0 {
        String::new()
    } else {
        format!("/admin/projects/{}/images", project.id)
    };
    let upload_note = if project.id == 0 {
        "<p class=\"hint\">Save this project once to enable pasted and dropped image uploads.</p>"
    } else {
        ""
    };
    format!(
        r#"<section class="admin-shell"><a class="text-link" href="/admin">&larr; Admin</a><h1>{}</h1><form method="post" action="{}" class="panel form-stack"><label>Title<input name="title" value="{}" required></label><label>Slug <span class="hint">(leave blank to generate from the title)</span><input name="slug" value="{}"></label><label>Summary<textarea name="summary" rows="3">{}</textarea></label><label>Cover image URL <span class="hint">(optional; copy one from the page image list)</span><input name="image_path" value="{}"></label><label>Body Markdown<textarea class="markdown-editor" data-image-upload="{}" name="body" rows="16">{}</textarea></label>{}<label class="switch"><input type="checkbox" name="published" {}><span class="switch-track" aria-hidden="true"></span><span>Published</span></label><button type="submit">Save project</button></form>{}</section>"#,
        escape_html(title),
        escape_html(action),
        escape_html(&project.title),
        escape_html(&project.slug),
        escape_html(&project.summary),
        escape_html(&project.image_path),
        escape_html(&image_upload),
        escape_html(&project.body),
        upload_note,
        if project.published { "checked" } else { "" },
        image_library(images),
    )
}

fn layout(title: &str, content: &str, admin: bool) -> String {
    let document_title = format!("{title} | George");
    layout_parts(
        title,
        &document_title,
        "Private site administration.",
        "",
        "",
        "George",
        "George",
        content,
        admin,
        "&copy; George",
        "",
    )
}

fn site_layout(
    state: &AppState,
    title: &str,
    description: &str,
    path: &str,
    social_image: Option<&str>,
    content: &str,
    admin: bool,
) -> Result<String, AppError> {
    let copyright = escape_html(&setting(state, "copyright_claim")?);
    let site_title = setting(state, "site_title")?;
    let author = setting(state, "author_name")?;
    let configured_social_image = setting(state, "social_image")?;
    let image = absolute_url(
        &state.site_url,
        social_image.unwrap_or(&configured_social_image),
    );
    let canonical = format!("{}{}", state.site_url, path);
    let document_title = if path == "/" {
        title.to_string()
    } else {
        format!("{title} | {site_title}")
    };
    let mut links = String::new();
    for link in list_footer_links(state)? {
        if !is_http_url(&link.url) {
            continue;
        }
        links.push_str(&footer_link_html(&link));
    }
    Ok(layout_parts(
        title,
        &document_title,
        description,
        &canonical,
        &image,
        &site_title,
        &author,
        content,
        admin,
        &copyright,
        &links,
    ))
}

#[allow(clippy::too_many_arguments)]
fn layout_parts(
    title: &str,
    document_title: &str,
    description: &str,
    canonical: &str,
    social_image: &str,
    site_title: &str,
    author: &str,
    content: &str,
    admin: bool,
    copyright: &str,
    links: &str,
) -> String {
    let admin_class = if admin { " admin-page" } else { "" };
    let robots = if admin {
        "noindex, nofollow"
    } else {
        "index, follow"
    };
    let image_meta = if social_image.is_empty() {
        String::new()
    } else {
        let image = escape_html(social_image);
        format!(
            r#"<meta property="og:image" content="{image}"><meta name="twitter:card" content="summary_large_image">"#
        )
    };
    let canonical_link = if canonical.is_empty() {
        String::new()
    } else {
        format!(
            r#"<link rel="canonical" href="{}">"#,
            escape_html(canonical)
        )
    };
    let public_meta = if canonical.is_empty() {
        String::new()
    } else {
        format!(
            r#"<meta property="og:url" content="{}">{}{}<script type="application/ld+json">{{"@context":"https://schema.org","@type":"WebSite","name":"{}","url":"{}","author":{{"@type":"Person","name":"{}"}}}}</script>"#,
            escape_html(canonical),
            image_meta,
            canonical_link,
            json_escape(site_title),
            json_escape(canonical),
            json_escape(author),
        )
    };
    let icon_assets = if admin || !links.is_empty() {
        format!(
            r#"<link rel="stylesheet" href="{}">"#,
            asset_url("/assets/vendor/font-awesome/css/font-awesome.min.css")
        )
    } else {
        String::new()
    };
    let editor_assets = if admin {
        format!(
            r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script><script src="{}" defer></script>"#,
            asset_url("/assets/vendor/easymde/easymde.min.css"),
            asset_url("/assets/vendor/easymde/easymde.min.js"),
            asset_url("/assets/editor.js"),
        )
    } else {
        String::new()
    };
    let theme_asset = asset_url("/assets/theme.js");
    let style_asset = asset_url("/assets/style.css");
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{}</title><meta name="description" content="{}"><meta name="author" content="{}"><meta name="robots" content="{}"><meta property="og:type" content="website"><meta property="og:title" content="{}"><meta property="og:description" content="{}"><meta name="twitter:card" content="summary">{}<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' rx='14' fill='%2328624f'/%3E%3Ctext x='32' y='44' text-anchor='middle' font-family='sans-serif' font-size='38' font-weight='700' fill='white'%3EG%3C/text%3E%3C/svg%3E"><script src="{}"></script><link rel="stylesheet" href="{}">{}{}</head><body class="{}"><div class="site-frame"><nav class="site-nav"><a class="brand" href="/">{}</a><div><a href="/projects">Projects</a>{}<button class="theme-toggle" type="button"><span class="sun-icon" aria-hidden="true">&#9788;</span><span class="moon-icon" aria-hidden="true">&#9790;</span></button></div></nav><main>{}</main><footer><span>{}</span><div>{}</div></footer></div></body></html>"#,
        escape_html(document_title),
        escape_html(description),
        escape_html(author),
        robots,
        escape_html(title),
        escape_html(description),
        public_meta,
        theme_asset,
        style_asset,
        icon_assets,
        editor_assets,
        admin_class,
        escape_html(site_title),
        if admin {
            r#"<a href="/admin">Admin</a>"#
        } else {
            ""
        },
        content,
        copyright,
        links,
    )
}

fn setting(state: &AppState, key: &str) -> Result<String, AppError> {
    Ok(state.db.lock().unwrap().query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |row| row.get(0),
    )?)
}

fn list_footer_links(state: &AppState) -> Result<Vec<FooterLink>, AppError> {
    let db = state.db.lock().unwrap();
    let mut statement =
        db.prepare("SELECT id, label, url FROM footer_links ORDER BY sort_order, id")?;
    let links = statement
        .query_map([], |row| {
            Ok(FooterLink {
                id: row.get(0)?,
                label: row.get(1)?,
                url: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(links)
}

fn markdown_to_html(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all())
        .filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));
    let mut output = String::new();
    html::push_html(&mut output, parser);
    format!("<div class=\"prose\">{output}</div>")
}

fn has_same_origin(headers: &HeaderMap) -> bool {
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

fn is_http_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn asset_url(path: &str) -> String {
    format!("{path}?v={ASSET_VERSION}")
}

fn footer_link_icon(url: &str) -> &'static str {
    let lower_url = url.to_ascii_lowercase();
    let host = lower_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(&lower_url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");

    if host_matches(host, "github.com") {
        "fa-github"
    } else if host_matches(host, "gitlab.com") {
        "fa-gitlab"
    } else if host_matches(host, "bitbucket.org") {
        "fa-bitbucket"
    } else if host_matches(host, "linkedin.com") {
        "fa-linkedin"
    } else if host_matches(host, "twitter.com") || host_matches(host, "x.com") {
        "fa-twitter"
    } else if host_matches(host, "facebook.com") {
        "fa-facebook"
    } else if host_matches(host, "instagram.com") {
        "fa-instagram"
    } else if host_matches(host, "youtube.com") || host == "youtu.be" {
        "fa-youtube"
    } else if host_matches(host, "reddit.com") {
        "fa-reddit"
    } else if host_matches(host, "stackoverflow.com") {
        "fa-stack-overflow"
    } else if host_matches(host, "stackexchange.com") {
        "fa-stack-exchange"
    } else if host_matches(host, "news.ycombinator.com") {
        "fa-hacker-news"
    } else if host_matches(host, "telegram.me") || host == "t.me" {
        "fa-telegram"
    } else if host_matches(host, "slack.com") {
        "fa-slack"
    } else if host_matches(host, "whatsapp.com") || host == "wa.me" {
        "fa-whatsapp"
    } else if host_matches(host, "twitch.tv") {
        "fa-twitch"
    } else if host_matches(host, "steamcommunity.com") || host_matches(host, "steampowered.com") {
        "fa-steam"
    } else if host_matches(host, "medium.com") {
        "fa-medium"
    } else if host_matches(host, "deviantart.com") {
        "fa-deviantart"
    } else if host_matches(host, "spotify.com") {
        "fa-spotify"
    } else if host_matches(host, "soundcloud.com") {
        "fa-soundcloud"
    } else if host_matches(host, "codepen.io") {
        "fa-codepen"
    } else if host_matches(host, "producthunt.com") {
        "fa-product-hunt"
    } else if host_matches(host, "trello.com") {
        "fa-trello"
    } else if host_matches(host, "dropbox.com") {
        "fa-dropbox"
    } else if host_matches(host, "flickr.com") {
        "fa-flickr"
    } else if host_matches(host, "pinterest.com") {
        "fa-pinterest"
    } else if host_matches(host, "tumblr.com") {
        "fa-tumblr"
    } else if host_matches(host, "vimeo.com") {
        "fa-vimeo"
    } else if host_matches(host, "paypal.com") {
        "fa-paypal"
    } else if host_matches(host, "rss.com") {
        "fa-rss"
    } else {
        "fa-external-link"
    }
}

fn footer_link_html(link: &FooterLink) -> String {
    format!(
        r#"<a href="{}" target="_blank" rel="me noopener noreferrer"><i class="fa {} footer-link-icon" aria-hidden="true"></i>{}</a>"#,
        escape_html(&link.url),
        footer_link_icon(&link.url),
        escape_html(&link.label),
    )
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn absolute_url(site_url: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{site_url}{path}")
    } else {
        path.to_string()
    }
}

fn json_escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn project_image(project: &Project) -> String {
    if project.image_path.is_empty() {
        return String::new();
    }
    format!(
        "<img class=\"project-image\" src=\"{}\" alt=\"\">",
        escape_html(&project.image_path)
    )
}

fn is_admin(headers: &HeaderMap, state: &AppState) -> bool {
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

fn random_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

fn normalized_slug(slug: &str, title: &str) -> String {
    let source = if slug.trim().is_empty() { title } else { slug };
    let mut output = String::new();
    let mut previous_dash = false;
    for character in source.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !output.is_empty() {
            output.push('-');
            previous_dash = true;
        }
    }
    output.trim_end_matches('-').to_string()
}

fn allowed_image_extension(file_name: &str) -> Option<&'static str> {
    match FilePath::new(file_name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        _ => None,
    }
}

fn image_bytes_match_extension(extension: &str, bytes: &[u8]) -> bool {
    match extension {
        "jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "webp" => {
            bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|kind| kind == b"WEBP")
        }
        _ => false,
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn not_found() -> Response {
    error_page(
        StatusCode::NOT_FOUND,
        "Not found",
        "404",
        "Page not found",
        "The page you were looking for does not exist.",
    )
}

fn internal_server_error() -> Response {
    error_page(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Server error",
        "500",
        "Something went wrong",
        "The server could not complete this request. Please try again shortly.",
    )
}

fn error_page(
    status: StatusCode,
    title: &str,
    code: &str,
    heading: &str,
    message: &str,
) -> Response {
    let content = format!(
        r#"<section class="error-page"><p class="eyebrow">{}</p><h1>{}</h1><p>{}</p><a class="button secondary" href="/">Return home</a></section>"#,
        escape_html(code),
        escape_html(heading),
        escape_html(message),
    );
    (status, Html(layout(title, &content, false))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_generated_from_a_title() {
        assert_eq!(
            normalized_slug("", "A Small Rust Site!"),
            "a-small-rust-site"
        );
    }

    #[test]
    fn explicit_slug_is_normalized() {
        assert_eq!(normalized_slug("  My_Project  ", "ignored"), "my-project");
    }

    #[test]
    fn uploads_only_accept_image_extensions() {
        assert_eq!(allowed_image_extension("photo.JPEG"), Some("jpg"));
        assert_eq!(allowed_image_extension("notes.html"), None);
    }

    #[test]
    fn uploads_require_matching_image_content() {
        assert!(image_bytes_match_extension("png", b"\x89PNG\r\n\x1a\n"));
        assert!(!image_bytes_match_extension("png", b"<script>"));
    }

    #[test]
    fn html_fields_are_escaped() {
        assert_eq!(
            escape_html("<script>'hello'</script>"),
            "&lt;script&gt;&#39;hello&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn raw_html_is_not_rendered_from_markdown() {
        let html = markdown_to_html("<script>alert('no')</script>\n\n**safe**");
        assert!(!html.contains("<script>"));
        assert!(html.contains("<strong>safe</strong>"));
    }

    #[test]
    fn footer_links_require_http_urls() {
        assert!(is_http_url("https://example.com"));
        assert!(is_http_url("http://example.com"));
        assert!(!is_http_url("javascript:alert(1)"));
    }

    #[test]
    fn footer_links_use_recognized_icons() {
        for (url, icon) in [
            ("https://github.com/example", "fa-github"),
            ("https://gitlab.com/example", "fa-gitlab"),
            ("https://bitbucket.org/example", "fa-bitbucket"),
            ("https://www.linkedin.com/in/example", "fa-linkedin"),
            ("https://x.com/example", "fa-twitter"),
            ("https://www.instagram.com/example", "fa-instagram"),
            ("https://youtube.com/@example", "fa-youtube"),
            ("https://reddit.com/u/example", "fa-reddit"),
            (
                "https://stackoverflow.com/users/1/example",
                "fa-stack-overflow",
            ),
            (
                "https://stackexchange.com/users/1/example",
                "fa-stack-exchange",
            ),
            (
                "https://news.ycombinator.com/user?id=example",
                "fa-hacker-news",
            ),
            ("https://t.me/example", "fa-telegram"),
            ("https://example.slack.com", "fa-slack"),
            ("https://wa.me/123456789", "fa-whatsapp"),
            ("https://twitch.tv/example", "fa-twitch"),
            ("https://steamcommunity.com/id/example", "fa-steam"),
            ("https://medium.com/@example", "fa-medium"),
            ("https://deviantart.com/example", "fa-deviantart"),
            ("https://open.spotify.com/user/example", "fa-spotify"),
            ("https://soundcloud.com/example", "fa-soundcloud"),
            ("https://codepen.io/example", "fa-codepen"),
            ("https://producthunt.com/@example", "fa-product-hunt"),
            ("https://trello.com/example", "fa-trello"),
            ("https://dropbox.com/example", "fa-dropbox"),
            ("https://flickr.com/photos/example", "fa-flickr"),
            ("https://pinterest.com/example", "fa-pinterest"),
            ("https://example.tumblr.com", "fa-tumblr"),
            ("https://vimeo.com/example", "fa-vimeo"),
            ("https://paypal.com/paypalme/example", "fa-paypal"),
        ] {
            assert_eq!(footer_link_icon(url), icon);
        }
        assert_eq!(
            footer_link_icon("https://example.com/projects"),
            "fa-external-link"
        );
        assert_eq!(
            footer_link_icon("https://notgithub.com/example"),
            "fa-external-link"
        );
    }

    #[test]
    fn public_footer_icons_load_the_local_icon_stylesheet() {
        let page = layout_parts(
            "Home",
            "Example | Home",
            "Description",
            "https://example.com/",
            "",
            "Example",
            "Example",
            "",
            false,
            "",
            r#"<a href="https://github.com/example"><i class="fa fa-github footer-link-icon"></i>GitHub</a>"#,
        );
        assert!(page.contains("/assets/vendor/font-awesome/css/font-awesome.min.css"));
    }

    #[test]
    fn footer_links_open_in_a_separate_tab_safely() {
        let html = footer_link_html(&FooterLink {
            id: 1,
            label: "GitHub".into(),
            url: "https://github.com/example".into(),
        });
        assert!(html.contains(r#"target="_blank""#));
        assert!(html.contains(r#"rel="me noopener noreferrer""#));
    }

    #[test]
    fn static_asset_urls_include_the_cache_busting_version() {
        assert_eq!(
            asset_url("/assets/theme.js"),
            format!("/assets/theme.js?v={ASSET_VERSION}")
        );
    }

    #[test]
    fn uploaded_social_images_become_absolute_urls() {
        assert_eq!(
            absolute_url("https://example.com", "/uploads/image.png"),
            "https://example.com/uploads/image.png"
        );
    }

    #[test]
    fn admin_posts_accept_same_origin_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("example.com"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.com"),
        );
        assert!(has_same_origin(&headers));
    }

    #[test]
    fn admin_posts_reject_cross_origin_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("example.com"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://attacker.example/form"),
        );
        assert!(!has_same_origin(&headers));
    }
}
