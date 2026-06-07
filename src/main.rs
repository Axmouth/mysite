use std::{
    collections::VecDeque,
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use rusqlite::Connection;
use tokio::{fs, net::TcpListener};
use tower_http::services::ServeDir;

mod db;
mod models;
mod render;
mod routes;
mod security;
mod template;
mod uploads;
mod utils;

use db::initialize_database;
pub(crate) use models::*;
use routes::*;
use security::security_headers;
use uploads::cleanup_orphaned_uploads;
use utils::random_token;

#[cfg(test)]
use axum::http::{HeaderMap, HeaderValue, header};
#[cfg(test)]
use render::{
    LayoutContext, absolute_url, asset_url, footer_link_html, footer_link_icon, layout_parts,
    markdown_to_html,
};
#[cfg(test)]
use security::has_same_origin;
#[cfg(test)]
use utils::{
    allowed_image_extension, escape_html, image_bytes_match_extension, is_http_url, normalized_slug,
};

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
        let page = layout_parts(LayoutContext {
            title: "Home",
            document_title: "Example | Home",
            description: "Description",
            canonical: "https://example.com/",
            social_image: "",
            site_title: "Example",
            author: "Example",
            content: "",
            admin: false,
            copyright: "",
            links: r#"<a href="https://github.com/example"><i class="fa fa-github footer-link-icon"></i>GitHub</a>"#,
        });
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
