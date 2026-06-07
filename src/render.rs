use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use pulldown_cmark::{Event, Options, Parser, html};

use crate::{
    ASSET_VERSION, AppError, AppState, FooterLink, OwnedImage, Project,
    db::{list_footer_links, setting},
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

pub(crate) struct LayoutContext<'a> {
    pub(crate) title: &'a str,
    pub(crate) document_title: &'a str,
    pub(crate) description: &'a str,
    pub(crate) canonical: &'a str,
    pub(crate) social_image: &'a str,
    pub(crate) site_title: &'a str,
    pub(crate) author: &'a str,
    pub(crate) content: &'a str,
    pub(crate) admin: bool,
    pub(crate) copyright: &'a str,
    pub(crate) links: &'a str,
}

pub(crate) fn layout(title: &str, content: &str, admin: bool) -> String {
    let document_title = format!("{title} | George");
    layout_parts(LayoutContext {
        title,
        document_title: &document_title,
        description: "Private site administration.",
        canonical: "",
        social_image: "",
        site_title: "George",
        author: "George",
        content,
        admin,
        copyright: "&copy; George",
        links: "",
    })
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
    Ok(layout_parts(LayoutContext {
        title,
        document_title: &document_title,
        description,
        canonical: &canonical,
        social_image: &image,
        site_title: &site_title,
        author: &author,
        content,
        admin,
        copyright: &copyright,
        links: &links,
    }))
}

pub(crate) fn layout_parts(context: LayoutContext<'_>) -> String {
    let LayoutContext {
        title,
        document_title,
        description,
        canonical,
        social_image,
        site_title,
        author,
        content,
        admin,
        copyright,
        links,
    } = context;
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

#[derive(Default)]
pub(crate) struct ProjectMetadata {
    pub(crate) date: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) tech: Vec<String>,
    pub(crate) links: Vec<ProjectMetadataLink>,
}

pub(crate) struct ProjectMetadataLink {
    pub(crate) label: &'static str,
    pub(crate) url: String,
}

pub(crate) fn split_project_markdown(markdown: &str) -> (ProjectMetadata, &str) {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return (ProjectMetadata::default(), markdown);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (ProjectMetadata::default(), markdown);
    };
    let metadata_block = &rest[..end];
    let body = &rest[end + "\n---\n".len()..];
    let mut metadata = ProjectMetadata::default();
    for line in metadata_block.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "date" => metadata.date = Some(value.to_string()),
            "status" => metadata.status = Some(value.to_string()),
            "tech" => {
                metadata.tech = value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }
            "source" | "repo" if is_http_url(value) => {
                metadata.links.push(ProjectMetadataLink {
                    label: "Source",
                    url: value.to_string(),
                });
            }
            "live" | "demo" if is_http_url(value) => {
                metadata.links.push(ProjectMetadataLink {
                    label: "Live",
                    url: value.to_string(),
                });
            }
            "docs" if is_http_url(value) => {
                metadata.links.push(ProjectMetadataLink {
                    label: "Docs",
                    url: value.to_string(),
                });
            }
            _ => {}
        }
    }
    (metadata, body)
}

pub(crate) fn project_metadata_html(metadata: &ProjectMetadata) -> String {
    let mut items = Vec::new();
    if let Some(date) = &metadata.date {
        items.push(format!("<span>{}</span>", escape_html(date)));
    }
    if let Some(status) = &metadata.status {
        items.push(format!("<span>{}</span>", escape_html(status)));
    }
    for tech in &metadata.tech {
        items.push(format!("<span>{}</span>", escape_html(tech)));
    }
    for link in &metadata.links {
        items.push(format!(
            r#"<a href="{}" target="_blank" rel="noopener noreferrer">{}</a>"#,
            escape_html(&link.url),
            escape_html(link.label),
        ));
    }
    if items.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="project-meta">{}</div>"#, items.join(""))
    }
}

pub(crate) fn asset_url(path: &str) -> String {
    format!("{path}?v={ASSET_VERSION}")
}

const FOOTER_ICON_RULES: &[(&[&str], &str)] = &[
    (&["github.com"], "fa-github"),
    (&["gitlab.com"], "fa-gitlab"),
    (&["bitbucket.org"], "fa-bitbucket"),
    (&["linkedin.com"], "fa-linkedin"),
    (&["twitter.com", "x.com"], "fa-twitter"),
    (&["facebook.com"], "fa-facebook"),
    (&["instagram.com"], "fa-instagram"),
    (&["youtube.com", "youtu.be"], "fa-youtube"),
    (&["reddit.com"], "fa-reddit"),
    (&["stackoverflow.com"], "fa-stack-overflow"),
    (&["stackexchange.com"], "fa-stack-exchange"),
    (&["news.ycombinator.com"], "fa-hacker-news"),
    (&["telegram.me", "t.me"], "fa-telegram"),
    (&["slack.com"], "fa-slack"),
    (&["whatsapp.com", "wa.me"], "fa-whatsapp"),
    (&["twitch.tv"], "fa-twitch"),
    (&["steamcommunity.com", "steampowered.com"], "fa-steam"),
    (&["medium.com"], "fa-medium"),
    (&["deviantart.com"], "fa-deviantart"),
    (&["spotify.com"], "fa-spotify"),
    (&["soundcloud.com"], "fa-soundcloud"),
    (&["codepen.io"], "fa-codepen"),
    (&["producthunt.com"], "fa-product-hunt"),
    (&["trello.com"], "fa-trello"),
    (&["dropbox.com"], "fa-dropbox"),
    (&["flickr.com"], "fa-flickr"),
    (&["pinterest.com"], "fa-pinterest"),
    (&["tumblr.com"], "fa-tumblr"),
    (&["vimeo.com"], "fa-vimeo"),
    (&["paypal.com"], "fa-paypal"),
    (&["rss.com"], "fa-rss"),
];

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

    for (domains, icon) in FOOTER_ICON_RULES {
        if domains.iter().any(|domain| host_matches(host, domain)) {
            return icon;
        }
    }
    "fa-external-link"
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
