use crate::api::models::{
    SeriesResponse, CreateSeriesRequest, UpdateSeriesRequest, BookResponse
};
use crate::core::error::{Result, TingError};
use crate::db::models::{Series, SeriesBook};
use crate::db::repository::Repository;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;
use super::AppState;

/// Handler for GET /api/v1/series - List all series
pub async fn list_series(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse> {
    let library_id = params.get("library_id").cloned();

    let series_list = if let Some(lid) = library_id {
        state.series_repo.find_by_library(&lid).await?
    } else {
        state.series_repo.find_all().await?
    };

    let mut response = Vec::new();
    for series in series_list {
        let mut s_res = SeriesResponse::from(series.clone());
        let books = state.series_repo.find_books_by_series(&series.id).await?;
        s_res.books = books.into_iter().map(|(b, _)| BookResponse::from(b)).collect();
        response.push(s_res);
    }

    Ok(Json(response))
}

/// Handler for GET /api/v1/series/:id - Get series by ID
pub async fn get_series(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    let series = state.series_repo.find_by_id(&id).await?
        .ok_or_else(|| TingError::NotFound(format!("Series with id {} not found", id)))?;

    let mut response = SeriesResponse::from(series.clone());
    let books = state.series_repo.find_books_by_series(&series.id).await?;
    response.books = books.into_iter().map(|(b, _)| BookResponse::from(b)).collect();

    Ok(Json(response))
}

/// Handler for POST /api/v1/series - Create a new series
pub async fn create_series(
    State(state): State<AppState>,
    Json(req): Json<CreateSeriesRequest>,
) -> Result<impl IntoResponse> {
    let series_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Default metadata from first book if not provided
    let mut author = req.author;
    let mut narrator = req.narrator;
    let mut cover_url = req.cover_url;
    let mut description = req.description;

    if !req.book_ids.is_empty() {
        if let Some(first_book) = state.book_repo.find_by_id(&req.book_ids[0]).await? {
            if author.is_none() { author = first_book.author; }
            if narrator.is_none() { narrator = first_book.narrator; }
            if cover_url.is_none() { cover_url = first_book.cover_url; }
            if description.is_none() { description = first_book.description; }
        }
    }

    let series = Series {
        id: series_id.clone(),
        library_id: req.library_id,
        title: req.title,
        author,
        narrator,
        cover_url,
        description,
        created_at: now.clone(),
        updated_at: now,
    };

    state.series_repo.create(&series).await?;

    // Add books
    for (idx, book_id) in req.book_ids.iter().enumerate() {
        state.series_repo.add_book(SeriesBook {
            series_id: series_id.clone(),
            book_id: book_id.clone(),
            book_order: idx as i32,
        }).await?;
        
        // Update metadata.json
        if let Err(e) = update_book_metadata_series(&state, book_id).await {
            tracing::warn!("Failed to update metadata.json for book {}: {}", book_id, e);
        }
    }

    let mut response = SeriesResponse::from(series);
    // Fetch added books to return full response
    let books = state.series_repo.find_books_by_series(&series_id).await?;
    response.books = books.into_iter().map(|(b, _)| BookResponse::from(b)).collect();

    Ok((StatusCode::CREATED, Json(response)))
}

/// Handler for PUT /api/v1/series/:id - Update a series
pub async fn update_series(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSeriesRequest>,
) -> Result<impl IntoResponse> {
    let existing_series = state.series_repo.find_by_id(&id).await?
        .ok_or_else(|| TingError::NotFound(format!("Series with id {} not found", id)))?;

    let updated_series = Series {
        id: existing_series.id,
        library_id: existing_series.library_id,
        title: req.title.clone().unwrap_or(existing_series.title),
        author: if req.author.is_some() { req.author } else { existing_series.author },
        narrator: if req.narrator.is_some() { req.narrator } else { existing_series.narrator },
        cover_url: if req.cover_url.is_some() { req.cover_url } else { existing_series.cover_url },
        description: if req.description.is_some() { req.description } else { existing_series.description },
        created_at: existing_series.created_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    state.series_repo.update(&updated_series).await?;

    // Update books if provided
    if let Some(book_ids) = req.book_ids {
        // We need to sync the books list.
        // Simplest way: remove all and re-add.
        // But SeriesRepository doesn't have "remove all books".
        // We can get current books, find diff.
        
        let current_books = state.series_repo.find_books_by_series(&id).await?;
        let current_ids: Vec<String> = current_books.iter().map(|(b, _)| b.id.clone()).collect();
        let mut affected_books = std::collections::HashSet::new();

        // Remove books that are not in new list
        for book_id in &current_ids {
            if !book_ids.contains(book_id) {
                state.series_repo.remove_book(&id, book_id).await?;
                affected_books.insert(book_id.clone());
            }
        }

        // Add/Update books
        for (idx, book_id) in book_ids.iter().enumerate() {
            state.series_repo.add_book(SeriesBook {
                series_id: id.clone(),
                book_id: book_id.clone(),
                book_order: idx as i32,
            }).await?;
            affected_books.insert(book_id.clone());
        }
        
        // Update metadata.json for all affected books
        for book_id in affected_books {
            if let Err(e) = update_book_metadata_series(&state, &book_id).await {
                tracing::warn!("Failed to update metadata.json for book {}: {}", book_id, e);
            }
        }
    } else {
        // If only title changed, we should update metadata for all books in series
        if req.title.is_some() {
            let books = state.series_repo.find_books_by_series(&id).await?;
            for (book, _) in books {
                if let Err(e) = update_book_metadata_series(&state, &book.id).await {
                    tracing::warn!("Failed to update metadata.json for book {}: {}", book.id, e);
                }
            }
        }
    }

    let mut response = SeriesResponse::from(updated_series);
    let books = state.series_repo.find_books_by_series(&id).await?;
    response.books = books.into_iter().map(|(b, _)| BookResponse::from(b)).collect();

    Ok(Json(response))
}

/// Handler for DELETE /api/v1/series/:id - Delete a series
pub async fn delete_series(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    if state.series_repo.find_by_id(&id).await?.is_none() {
        return Err(TingError::NotFound(format!("Series with id {} not found", id)));
    }
    
    // Get books before deletion to update metadata
    let books = state.series_repo.find_books_by_series(&id).await?;

    state.series_repo.delete(&id).await?;
    
    // Update metadata.json for affected books
    for (book, _) in books {
        if let Err(e) = update_book_metadata_series(&state, &book.id).await {
            tracing::warn!("Failed to update metadata.json for book {}: {}", book.id, e);
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Helper function to update metadata.json for a book
async fn update_book_metadata_series(state: &AppState, book_id: &str) -> Result<()> {
    use crate::db::repository::Repository;
    
    if let Some(book) = state.book_repo.find_by_id(book_id).await? {
        let path = std::path::Path::new(&book.path);
        
        // Read existing metadata
        if let Ok(Some(mut metadata)) = crate::core::metadata_writer::read_metadata_json(path) {
            // Fetch all series for this book
            let series_list = state.series_repo.find_series_by_book(book_id).await?;
            let series_titles: Vec<String> = series_list.into_iter().map(|s| s.title).collect();
            
            // Update series
            metadata.series = series_titles;
            
            // Write back
            if let Err(e) = crate::core::metadata_writer::write_metadata_json(path, &metadata) {
                tracing::warn!("Failed to write metadata.json: {}", e);
            }
        }
    }
    Ok(())
}
