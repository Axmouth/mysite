use rusqlite::{Connection, OptionalExtension, params};

use crate::{AppError, AppState, ContactMessage, DEFAULT_HOME, FooterLink, OwnedImage, Project};

pub(crate) struct ProjectMutation<'a> {
    pub(crate) slug: &'a str,
    pub(crate) title: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) body: &'a str,
    pub(crate) image_path: &'a str,
    pub(crate) published: bool,
    pub(crate) featured: bool,
}

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

        CREATE TABLE IF NOT EXISTS contact_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT NOT NULL DEFAULT '',
            message TEXT NOT NULL,
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

pub(crate) fn update_settings(
    state: &AppState,
    values: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<(), AppError> {
    let db = state.db.lock().unwrap();
    for (key, value) in values {
        db.execute(
            "UPDATE settings SET value = ?1 WHERE key = ?2",
            params![value, key],
        )?;
    }
    Ok(())
}

pub(crate) fn create_footer_link(state: &AppState, label: &str, url: &str) -> Result<(), AppError> {
    state.db.lock().unwrap().execute(
        "INSERT INTO footer_links (label, url, sort_order) VALUES (?1, ?2, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM footer_links))",
        params![label, url],
    )?;
    Ok(())
}

pub(crate) fn delete_footer_link(state: &AppState, id: i64) -> Result<(), AppError> {
    state
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM footer_links WHERE id = ?1", [id])?;
    Ok(())
}

pub(crate) fn home_markdown(state: &AppState) -> Result<String, AppError> {
    setting(state, "home_markdown")
}

pub(crate) fn update_home_markdown(state: &AppState, markdown: &str) -> Result<(), AppError> {
    state.db.lock().unwrap().execute(
        "UPDATE settings SET value = ?1 WHERE key = 'home_markdown'",
        [markdown],
    )?;
    Ok(())
}

pub(crate) fn create_project(
    state: &AppState,
    project: &ProjectMutation<'_>,
) -> rusqlite::Result<usize> {
    state.db.lock().unwrap().execute(
        "INSERT INTO projects (slug, title, summary, body, image_path, published, featured) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            project.slug,
            project.title,
            project.summary,
            project.body,
            project.image_path,
            project.published,
            project.featured
        ],
    )
}

pub(crate) fn update_project(
    state: &AppState,
    id: i64,
    project: &ProjectMutation<'_>,
) -> rusqlite::Result<usize> {
    state.db.lock().unwrap().execute(
        "UPDATE projects SET slug = ?1, title = ?2, summary = ?3, body = ?4, image_path = ?5, published = ?6, featured = ?7, updated_at = CURRENT_TIMESTAMP WHERE id = ?8",
        params![
            project.slug,
            project.title,
            project.summary,
            project.body,
            project.image_path,
            project.published,
            project.featured,
            id
        ],
    )
}

pub(crate) fn delete_project_and_images(state: &AppState, id: i64) -> Result<(), AppError> {
    let db = state.db.lock().unwrap();
    db.execute(
        "DELETE FROM images WHERE owner_type = 'project' AND owner_id = ?1",
        [id],
    )?;
    db.execute("DELETE FROM projects WHERE id = ?1", [id])?;
    Ok(())
}

pub(crate) fn create_image_record(
    state: &AppState,
    file_name: &str,
    original_name: &str,
    owner_type: &str,
    owner_id: Option<i64>,
) -> Result<(), AppError> {
    state.db.lock().unwrap().execute(
        "INSERT INTO images (file_name, original_name, owner_type, owner_id) VALUES (?1, ?2, ?3, ?4)",
        params![file_name, original_name, owner_type, owner_id],
    )?;
    Ok(())
}

pub(crate) fn image_file_name_by_id(state: &AppState, id: i64) -> Result<Option<String>, AppError> {
    Ok(state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT file_name FROM images WHERE id = ?1", [id], |row| {
            row.get::<_, String>(0)
        })
        .optional()?)
}

pub(crate) fn delete_image_record(state: &AppState, id: i64) -> Result<(), AppError> {
    state
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM images WHERE id = ?1", [id])?;
    Ok(())
}

pub(crate) fn orphaned_project_image_names(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT file_name FROM images WHERE owner_type = 'project' AND owner_id NOT IN (SELECT id FROM projects)",
    )?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect()
}

pub(crate) fn delete_orphaned_project_image_records(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM images WHERE owner_type = 'project' AND owner_id NOT IN (SELECT id FROM projects)",
        [],
    )?;
    Ok(())
}

pub(crate) fn tracked_image_file_names(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare("SELECT file_name FROM images")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect()
}

pub(crate) fn create_contact_message(
    state: &AppState,
    name: &str,
    email: &str,
    message: &str,
) -> Result<(), AppError> {
    state.db.lock().unwrap().execute(
        "INSERT INTO contact_messages (name, email, message) VALUES (?1, ?2, ?3)",
        params![name, email, message],
    )?;
    Ok(())
}

pub(crate) fn list_contact_messages(state: &AppState) -> Result<Vec<ContactMessage>, AppError> {
    let db = state.db.lock().unwrap();
    let mut statement = db.prepare(
        "SELECT id, name, email, message, created_at FROM contact_messages ORDER BY created_at DESC, id DESC",
    )?;
    let messages = statement
        .query_map([], |row| {
            Ok(ContactMessage {
                id: row.get(0)?,
                name: row.get(1)?,
                email: row.get(2)?,
                message: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(messages)
}

pub(crate) fn delete_contact_message(state: &AppState, id: i64) -> Result<(), AppError> {
    state
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM contact_messages WHERE id = ?1", [id])?;
    Ok(())
}
