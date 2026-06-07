use std::path::Path as FilePath;

use axum::{
    Json,
    extract::Multipart,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rusqlite::Connection;
use serde::Serialize;
use tokio::fs;
use uuid::Uuid;

use crate::{
    AppError, AppState,
    db::{
        create_image_record, delete_orphaned_project_image_records, orphaned_project_image_names,
        tracked_image_file_names,
    },
    utils::{allowed_image_extension, image_bytes_match_extension},
};

pub(crate) async fn cleanup_orphaned_uploads(
    db: &Connection,
    uploads_dir: &FilePath,
) -> Result<(), AppError> {
    for file_name in orphaned_project_image_names(db)? {
        remove_upload_file(uploads_dir, &file_name).await?;
    }
    delete_orphaned_project_image_records(db)?;

    let tracked = tracked_image_file_names(db)?;
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

pub(crate) async fn remove_upload_file(
    uploads_dir: &FilePath,
    file_name: &str,
) -> Result<(), AppError> {
    match fs::remove_file(uploads_dir.join(file_name)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
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

pub(crate) async fn store_image(
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
    if create_image_record(state, &stored_name, &original_name, owner_type, owner_id).is_err() {
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
