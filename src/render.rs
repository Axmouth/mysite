use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use pulldown_cmark::{Event, Options, Parser, html};

use crate::{
    ASSET_VERSION, AppError, AppState, FooterLink, OwnedImage, Project, list_footer_links, setting,
    template,
    utils::{escape_html, is_http_url},
};

pub(crate) fn image_library(images: &[OwnedImage]) -> String {
    let mut cards = String::new();
    for image in images {
        let path = format!("/uploads/{}", image.file_name);
        cards.push_str(&template::render(
            include_str!("../templates/admin/image_card.html"),
            &[
                ("id", image.id.to_string()),
                ("name", escape_html(&image.original_name)),
                ("path", escape_html(&path)),
            ],
        ));
    }
    template::render(
        include_str!("../templates/admin/image_library.html"),
        &[("cards", cards)],
    )
}

pub(crate) fn project_form(
    heading: &str,
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
        featured: false,
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
    template::render(
        include_str!("../templates/admin/project_form.html"),
        &[
            ("heading", escape_html(heading)),
            ("action", escape_html(action)),
            ("title", escape_html(&project.title)),
            ("slug", escape_html(&project.slug)),
            ("summary", escape_html(&project.summary)),
            ("image_path", escape_html(&project.image_path)),
            ("image_upload", escape_html(&image_upload)),
            ("body", escape_html(&project.body)),
            ("upload_note", upload_note.into()),
            (
                "published",
                if project.published { "checked" } else { "" }.into(),
            ),
            (
                "featured",
                if project.featured { "checked" } else { "" }.into(),
            ),
            ("images", image_library(images)),
        ],
    )
}

pub(crate) fn layout(title: &str, content: &str, admin: bool) -> String {
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

pub(crate) fn site_layout(
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
pub(crate) fn layout_parts(
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
    template::render(
        include_str!("../templates/layout.html"),
        &[
            ("document_title", escape_html(document_title)),
            ("description", escape_html(description)),
            ("author", escape_html(author)),
            ("robots", robots.into()),
            ("title", escape_html(title)),
            ("public_meta", public_meta),
            ("theme_asset", asset_url("/assets/theme.js")),
            ("style_asset", asset_url("/assets/style.css")),
            ("icon_assets", icon_assets),
            ("editor_assets", editor_assets),
            ("body_class", admin_class.into()),
            ("site_title", escape_html(site_title)),
            (
                "admin_link",
                if admin {
                    r#"<a href="/admin">Admin</a>"#.into()
                } else {
                    String::new()
                },
            ),
            ("content", content.into()),
            ("copyright", copyright.into()),
            ("links", links.into()),
        ],
    )
}

pub(crate) fn markdown_to_html(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all())
        .filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));
    let mut output = String::new();
    html::push_html(&mut output, parser);
    format!("<div class=\"prose\">{output}</div>")
}

pub(crate) fn asset_url(path: &str) -> String {
    format!("{path}?v={ASSET_VERSION}")
}

pub(crate) fn footer_link_icon(url: &str) -> &'static str {
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

pub(crate) fn footer_link_html(link: &FooterLink) -> String {
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

pub(crate) fn absolute_url(site_url: &str, path: &str) -> String {
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

pub(crate) fn project_image(project: &Project) -> String {
    if project.image_path.is_empty() {
        return String::new();
    }
    format!(
        "<img class=\"project-image\" src=\"{}\" alt=\"\">",
        escape_html(&project.image_path)
    )
}

pub(crate) fn not_found() -> Response {
    error_page(
        StatusCode::NOT_FOUND,
        "Not found",
        "404",
        "Page not found",
        "The page you were looking for does not exist.",
    )
}

pub(crate) fn internal_server_error() -> Response {
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
    let content = template::render(
        include_str!("../templates/error.html"),
        &[
            ("code", escape_html(code)),
            ("heading", escape_html(heading)),
            ("message", escape_html(message)),
        ],
    );
    (status, Html(layout(title, &content, false))).into_response()
}
