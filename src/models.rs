use std::{collections::VecDeque, path::PathBuf, sync::Mutex, time::Instant};

use axum::response::{IntoResponse, Response};
use rusqlite::Connection;

use crate::render::internal_server_error;

pub(crate) struct AppState {
    pub(crate) db: Mutex<Connection>,
    pub(crate) admin_password: String,
    pub(crate) session_token: String,
    pub(crate) uploads_dir: PathBuf,
    pub(crate) secure_cookie: bool,
    pub(crate) site_url: String,
    pub(crate) login_failures: Mutex<VecDeque<Instant>>,
}

#[derive(Clone)]
pub(crate) struct Project {
    pub(crate) id: i64,
    pub(crate) slug: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) body: String,
    pub(crate) image_path: String,
    pub(crate) published: bool,
    pub(crate) featured: bool,
}

pub(crate) struct FooterLink {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) url: String,
}

pub(crate) struct OwnedImage {
    pub(crate) id: i64,
    pub(crate) file_name: String,
    pub(crate) original_name: String,
}

pub(crate) struct ContactMessage {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) message: String,
    pub(crate) created_at: String,
}

#[derive(Debug)]
pub(crate) struct AppError(pub(crate) String);

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
