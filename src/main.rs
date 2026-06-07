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
    extract::{DefaultBodyLimit, Form, Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::{fs, net::TcpListener};
use tower_http::services::ServeDir;
use uuid::Uuid;

mod db;
mod models;
mod render;
mod security;
mod template;
mod utils;

use db::*;
pub(crate) use models::*;
use render::*;
use security::*;
use utils::*;

const DEFAULT_HOME: &str = r#"# Hello, I'm George.

This is a small corner of the web for my work and notes.

[See my projects](/projects)
"#;
pub(crate) const ASSET_VERSION: &str = "20260602-1";

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
    let mut cards = String::new();
    for project in projects {
        let slug = escape_html(&project.slug);
        cards.push_str(&template::render(
            include_str!("../templates/public/project_card.html"),
            &[
                ("slug", slug),
                ("image", project_image(&project)),
                (
                    "featured_label",
                    if project.featured {
                        r#"<p class="eyebrow">Featured</p>"#.into()
                    } else {
                        String::new()
                    },
                ),
                ("title", escape_html(&project.title)),
                ("summary", escape_html(&project.summary)),
            ],
        ));
    }
    let content = template::render(
        include_str!("../templates/public/project_list.html"),
        &[("projects", cards)],
    );
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

    let escaped_slug = escape_html(&project.slug);
    let content = template::render(
        include_str!("../templates/public/project_detail.html"),
        &[
            ("slug", escaped_slug),
            ("title", escape_html(&project.title)),
            ("summary", escape_html(&project.summary)),
            ("image", project_image(&project)),
            ("body", markdown_to_html(&project.body)),
        ],
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
        include_str!("../templates/admin/login.html"),
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
                include_str!("../templates/admin/login_error.html"),
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
        rows.push_str(&template::render(
            include_str!("../templates/admin/project_row.html"),
            &[
                ("id", project.id.to_string()),
                ("title", escape_html(&project.title)),
                (
                    "status",
                    if project.published {
                        "Published"
                    } else {
                        "Draft"
                    }
                    .into(),
                ),
                (
                    "featured",
                    if project.featured { ", Featured" } else { "" }.into(),
                ),
            ],
        ));
    }
    let content = template::render(
        include_str!("../templates/admin/dashboard.html"),
        &[("rows", rows)],
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
        links_html.push_str(&template::render(
            include_str!("../templates/admin/settings_link.html"),
            &[
                ("id", link.id.to_string()),
                ("label", escape_html(&link.label)),
                ("url", escape_html(&link.url)),
            ],
        ));
    }
    let content = template::render(
        include_str!("../templates/admin/settings.html"),
        &[
            ("site_title", escape_html(&site_title)),
            ("home_seo_title", escape_html(&home_seo_title)),
            ("author_name", escape_html(&author_name)),
            ("site_description", escape_html(&site_description)),
            ("social_image", escape_html(&social_image)),
            ("copyright", escape_html(&copyright)),
            ("links", links_html),
        ],
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
    let content = template::render(
        include_str!("../templates/admin/home_form.html"),
        &[
            ("markdown", escape_html(&markdown)),
            ("images", image_library(&images)),
        ],
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
    featured: Option<String>,
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
        "INSERT INTO projects (slug, title, summary, body, image_path, published, featured) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![slug, form.title, form.summary, form.body, form.image_path, form.published.is_some(), form.featured.is_some()],
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
        "UPDATE projects SET slug = ?1, title = ?2, summary = ?3, body = ?4, image_path = ?5, published = ?6, featured = ?7, updated_at = CURRENT_TIMESTAMP WHERE id = ?8",
        params![slug, form.title, form.summary, form.body, form.image_path, form.published.is_some(), form.featured.is_some(), id],
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
