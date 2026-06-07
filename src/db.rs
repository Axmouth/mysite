use rusqlite::{Connection, OptionalExtension, params};

use crate::{AppError, AppState, DEFAULT_HOME, FooterLink, OwnedImage, Project};

pub(crate) fn initialize_database(db: &Connection) -> rusqlite::Result<()> {
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
            featured INTEGER NOT NULL DEFAULT 0,
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
    add_column_if_missing(
        db,
        "projects",
        "featured",
        "ALTER TABLE projects ADD COLUMN featured INTEGER NOT NULL DEFAULT 0",
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

pub(crate) fn list_projects(
    state: &AppState,
    only_published: bool,
) -> Result<Vec<Project>, AppError> {
    let db = state.db.lock().unwrap();
    let query = if only_published {
        "SELECT id, slug, title, summary, body, image_path, published, featured FROM projects WHERE published = 1 ORDER BY featured DESC, created_at DESC"
    } else {
        "SELECT id, slug, title, summary, body, image_path, published, featured FROM projects ORDER BY featured DESC, created_at DESC"
    };
    let mut statement = db.prepare(query)?;
    let projects = statement
        .query_map([], project_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(projects)
}

pub(crate) fn find_project_by_slug(
    state: &AppState,
    slug: &str,
) -> Result<Option<Project>, AppError> {
    let project = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id, slug, title, summary, body, image_path, published, featured FROM projects WHERE slug = ?1",
            [slug],
            project_from_row,
        )
        .optional()?;
    Ok(project)
}

pub(crate) fn find_project_by_id(state: &AppState, id: i64) -> Result<Option<Project>, AppError> {
    let project = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id, slug, title, summary, body, image_path, published, featured FROM projects WHERE id = ?1",
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
        featured: row.get(7)?,
    })
}

fn add_column_if_missing(
    db: &Connection,
    table: &str,
    column: &str,
    statement: &str,
) -> rusqlite::Result<()> {
    let mut columns = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        db.execute(statement, [])?;
    }
    Ok(())
}

pub(crate) fn list_images(
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

pub(crate) fn setting(state: &AppState, key: &str) -> Result<String, AppError> {
    Ok(state.db.lock().unwrap().query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |row| row.get(0),
    )?)
}

pub(crate) fn list_footer_links(state: &AppState) -> Result<Vec<FooterLink>, AppError> {
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
