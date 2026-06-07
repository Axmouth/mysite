use std::{
    fmt::Write as _,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{Form, Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use rusqlite::params;
use serde::Deserialize;

use crate::{
    AppError, AppState,
    db::{
        find_project_by_id, find_project_by_slug, list_footer_links, list_images, list_projects,
        setting,
    },
    render::{
        image_library, layout, markdown_to_html, not_found, project_form, project_image,
        site_layout,
    },
    security::is_admin,
    template,
    uploads::{delete_image_record, image_file_name_by_id, remove_upload_file, store_image},
    utils::{escape_html, is_http_url, normalized_slug},
};

pub(crate) async fn home(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
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

pub(crate) async fn project_list(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, AppError> {
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

pub(crate) async fn project_detail(
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

pub(crate) async fn healthz() -> &'static str {
    "ok"
}

pub(crate) async fn robots_txt(State(state): State<Arc<AppState>>) -> String {
    format!(
        "User-agent: *\nAllow: /\nDisallow: /admin\nSitemap: {}/sitemap.xml\n",
        state.site_url
    )
}

pub(crate) async fn sitemap_xml(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
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

pub(crate) async fn fallback_not_found() -> Response {
    not_found()
}

#[cfg(debug_assertions)]
pub(crate) async fn test_internal_server_error() -> Response {
    AppError("intentional debug error".into()).into_response()
}

pub(crate) async fn login_page() -> Html<String> {
    Html(layout(
        "Admin login",
        include_str!("../templates/admin/login.html"),
        false,
    ))
}

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    password: String,
}

pub(crate) async fn login(
    State(state): State<Arc<AppState>>,
    Form(form): Form<LoginForm>,
) -> Response {
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

pub(crate) async fn logout() -> Response {
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("admin_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    response
}

pub(crate) async fn admin_dashboard(
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

pub(crate) async fn admin_settings(
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
pub(crate) struct SettingsForm {
    site_title: String,
    home_seo_title: String,
    author_name: String,
    site_description: String,
    social_image: String,
    copyright_claim: String,
}

pub(crate) async fn update_settings(
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
pub(crate) struct FooterLinkForm {
    label: String,
    url: String,
}

pub(crate) async fn create_footer_link(
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

pub(crate) async fn delete_footer_link(
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

pub(crate) async fn admin_home(
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
pub(crate) struct HomeForm {
    markdown: String,
}

pub(crate) async fn update_home(
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

pub(crate) async fn new_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
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
pub(crate) struct ProjectForm {
    title: String,
    slug: String,
    summary: String,
    body: String,
    image_path: String,
    published: Option<String>,
    featured: Option<String>,
}

pub(crate) async fn create_project(
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

pub(crate) async fn edit_project(
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

pub(crate) async fn update_project(
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

pub(crate) async fn delete_project(
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

pub(crate) async fn upload_home_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if !is_admin(&headers, &state) {
        return Redirect::to("/admin/login").into_response();
    }
    store_image(&state, multipart, "home", None).await
}

pub(crate) async fn upload_project_image(
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

pub(crate) async fn delete_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !is_admin(&headers, &state) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    if let Some(file_name) = image_file_name_by_id(&state, id)? {
        remove_upload_file(&state.uploads_dir, &file_name).await?;
        delete_image_record(&state, id)?;
    }
    Ok(Redirect::to("/admin").into_response())
}
