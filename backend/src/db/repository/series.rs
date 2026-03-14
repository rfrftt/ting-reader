use crate::core::error::{Result, TingError};
use crate::db::manager::DatabaseManager;
use crate::db::models::{Series, SeriesBook, Book};
use crate::db::repository::base::Repository;
use async_trait::async_trait;
use rusqlite::OptionalExtension;
use std::sync::Arc;

/// Repository for Series entities
pub struct SeriesRepository {
    db: Arc<DatabaseManager>,
}

impl SeriesRepository {
    /// Create a new SeriesRepository
    pub fn new(db: Arc<DatabaseManager>) -> Self {
        Self { db }
    }

    /// Find series by library ID
    pub async fn find_by_library(&self, library_id: &str) -> Result<Vec<Series>> {
        let library_id = library_id.to_string();
        self.db.execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, library_id, title, author, narrator, cover_url, description, created_at, updated_at \
                 FROM series WHERE library_id = ? ORDER BY created_at DESC"
            ).map_err(TingError::DatabaseError)?;

            let series = stmt.query_map([&library_id], |row| {
                Ok(Series {
                    id: row.get(0)?,
                    library_id: row.get(1)?,
                    title: row.get(2)?,
                    author: row.get(3)?,
                    narrator: row.get(4)?,
                    cover_url: row.get(5)?,
                    description: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            }).map_err(TingError::DatabaseError)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(TingError::DatabaseError)?;

            Ok(series)
        }).await
    }

    /// Find series by title and library
    pub async fn find_by_title_and_library(&self, title: &str, library_id: &str) -> Result<Option<Series>> {
        let title = title.to_string();
        let library_id = library_id.to_string();
        self.db.execute(move |conn| {
            conn.query_row(
                "SELECT id, library_id, title, author, narrator, cover_url, description, created_at, updated_at \
                 FROM series WHERE title = ? AND library_id = ?",
                rusqlite::params![&title, &library_id],
                |row| {
                    Ok(Series {
                        id: row.get(0)?,
                        library_id: row.get(1)?,
                        title: row.get(2)?,
                        author: row.get(3)?,
                        narrator: row.get(4)?,
                        cover_url: row.get(5)?,
                        description: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                }
            ).optional()
            .map_err(TingError::DatabaseError)
        }).await
    }

    /// Find series for a book
    pub async fn find_series_by_book(&self, book_id: &str) -> Result<Vec<Series>> {
        let book_id = book_id.to_string();
        self.db.execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.library_id, s.title, s.author, s.narrator, s.cover_url, s.description, s.created_at, s.updated_at \
                 FROM series s \
                 JOIN series_books sb ON s.id = sb.series_id \
                 WHERE sb.book_id = ?"
            ).map_err(TingError::DatabaseError)?;

            let series = stmt.query_map([&book_id], |row| {
                Ok(Series {
                    id: row.get(0)?,
                    library_id: row.get(1)?,
                    title: row.get(2)?,
                    author: row.get(3)?,
                    narrator: row.get(4)?,
                    cover_url: row.get(5)?,
                    description: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            }).map_err(TingError::DatabaseError)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(TingError::DatabaseError)?;

            Ok(series)
        }).await
    }



    /// Find books in a series
    pub async fn find_books_by_series(&self, series_id: &str) -> Result<Vec<(Book, i32)>> {
        let series_id = series_id.to_string();
        self.db.execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT b.id, b.library_id, b.title, b.author, b.narrator, b.cover_url, b.theme_color, \
                 b.description, b.skip_intro, b.skip_outro, b.path, b.hash, b.tags, b.created_at, \
                 b.manual_corrected, b.match_pattern, b.chapter_regex, sb.book_order \
                 FROM books b \
                 JOIN series_books sb ON b.id = sb.book_id \
                 WHERE sb.series_id = ? \
                 ORDER BY sb.book_order ASC"
            ).map_err(TingError::DatabaseError)?;

            let books = stmt.query_map([&series_id], |row| {
                let book = Book {
                    id: row.get(0)?,
                    library_id: row.get(1)?,
                    title: row.get(2)?,
                    author: row.get(3)?,
                    narrator: row.get(4)?,
                    cover_url: row.get(5)?,
                    theme_color: row.get(6)?,
                    description: row.get(7)?,
                    skip_intro: row.get(8)?,
                    skip_outro: row.get(9)?,
                    path: row.get(10)?,
                    hash: row.get(11)?,
                    tags: row.get(12)?,
                    created_at: row.get(13)?,
                    manual_corrected: row.get(14).unwrap_or(0),
                    match_pattern: row.get(15).unwrap_or(None),
                    chapter_regex: row.get(16).unwrap_or(None),
                    genre: None,
                };
                let order: i32 = row.get(17)?;
                Ok((book, order))
            }).map_err(TingError::DatabaseError)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(TingError::DatabaseError)?;

            Ok(books)
        }).await
    }

    /// Add a book to a series
    pub async fn add_book(&self, series_book: SeriesBook) -> Result<()> {
        self.db.execute(move |conn| {
            conn.execute(
                "INSERT INTO series_books (series_id, book_id, book_order) VALUES (?, ?, ?) \
                 ON CONFLICT(series_id, book_id) DO UPDATE SET book_order = excluded.book_order",
                rusqlite::params![&series_book.series_id, &series_book.book_id, series_book.book_order],
            ).map_err(TingError::DatabaseError)?;
            Ok(())
        }).await
    }

    /// Remove a book from a series
    pub async fn remove_book(&self, series_id: &str, book_id: &str) -> Result<()> {
        let series_id = series_id.to_string();
        let book_id = book_id.to_string();
        self.db.execute(move |conn| {
            conn.execute(
                "DELETE FROM series_books WHERE series_id = ? AND book_id = ?",
                [&series_id, &book_id],
            ).map_err(TingError::DatabaseError)?;
            Ok(())
        }).await
    }

    /// Update book orders in a series
    pub async fn update_book_orders(&self, series_id: &str, orders: Vec<(String, i32)>) -> Result<()> {
        let series_id = series_id.to_string();
        self.db.transaction(move |tx| {
            for (book_id, order) in orders {
                tx.execute(
                    "UPDATE series_books SET book_order = ? WHERE series_id = ? AND book_id = ?",
                    rusqlite::params![order, &series_id, &book_id],
                ).map_err(TingError::DatabaseError)?;
            }
            Ok(())
        }).await
    }
}

#[async_trait]
impl Repository<Series> for SeriesRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<Series>> {
        let id = id.to_string();
        self.db.execute(move |conn| {
            conn.query_row(
                "SELECT id, library_id, title, author, narrator, cover_url, description, created_at, updated_at \
                 FROM series WHERE id = ?",
                [&id],
                |row| {
                    Ok(Series {
                        id: row.get(0)?,
                        library_id: row.get(1)?,
                        title: row.get(2)?,
                        author: row.get(3)?,
                        narrator: row.get(4)?,
                        cover_url: row.get(5)?,
                        description: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                }
            ).optional()
            .map_err(TingError::DatabaseError)
        }).await
    }

    async fn find_all(&self) -> Result<Vec<Series>> {
        self.db.execute(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, library_id, title, author, narrator, cover_url, description, created_at, updated_at \
                 FROM series ORDER BY created_at DESC"
            ).map_err(TingError::DatabaseError)?;

            let series = stmt.query_map([], |row| {
                Ok(Series {
                    id: row.get(0)?,
                    library_id: row.get(1)?,
                    title: row.get(2)?,
                    author: row.get(3)?,
                    narrator: row.get(4)?,
                    cover_url: row.get(5)?,
                    description: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            }).map_err(TingError::DatabaseError)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(TingError::DatabaseError)?;

            Ok(series)
        }).await
    }

    async fn create(&self, series: &Series) -> Result<()> {
        let series = series.clone();
        self.db.execute(move |conn| {
            conn.execute(
                "INSERT INTO series (id, library_id, title, author, narrator, cover_url, description) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    &series.id,
                    &series.library_id,
                    &series.title,
                    &series.author,
                    &series.narrator,
                    &series.cover_url,
                    &series.description,
                ],
            ).map_err(TingError::DatabaseError)?;
            Ok(())
        }).await
    }

    async fn update(&self, series: &Series) -> Result<()> {
        let series = series.clone();
        self.db.execute(move |conn| {
            conn.execute(
                "UPDATE series SET library_id = ?, title = ?, author = ?, narrator = ?, \
                 cover_url = ?, description = ?, updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
                rusqlite::params![
                    &series.library_id,
                    &series.title,
                    &series.author,
                    &series.narrator,
                    &series.cover_url,
                    &series.description,
                    &series.id,
                ],
            ).map_err(TingError::DatabaseError)?;
            Ok(())
        }).await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.execute(move |conn| {
            conn.execute("DELETE FROM series WHERE id = ?", [&id])
                .map_err(TingError::DatabaseError)?;
            Ok(())
        }).await
    }
}
