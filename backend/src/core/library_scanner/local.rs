use super::{LibraryScanner, ScanResult, ScanStatus, MetadataSource, STANDARD_EXTENSIONS};
use crate::core::error::{Result, TingError};
use crate::db::models::Chapter;
use crate::core::nfo_manager::BookMetadata;
use crate::db::repository::Repository;
use crate::plugin::manager::FormatMethod;
use crate::plugin::types::PluginType;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use walkdir::WalkDir;
use tracing::{debug, info, warn};
use uuid::Uuid;
use sha2::{Digest, Sha256};
use std::io::Read;

impl LibraryScanner {
    /// Scan a local library
    pub(crate) async fn scan_local_library(
        &self,
        library_id: &str,
        path: &Path,
        task_id: Option<&str>,
        last_scanned: Option<chrono::DateTime<chrono::Utc>>,
        scraper_config: &crate::db::models::ScraperConfig,
    ) -> Result<ScanResult> {
        let mut scan_result = ScanResult::default();
        scan_result.start_time = Some(std::time::Instant::now());
        
        self.update_progress(task_id, "Scanning local directories...".to_string()).await;

        // Get all supported extensions dynamically
        let supported_extensions = self.get_supported_extensions().await;
        
        // 1. Recursively find all audio files and group them by directory
        let mut dir_groups: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(ext) = entry_path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if supported_extensions.contains(&ext_str) {
                        if let Some(parent) = entry_path.parent() {
                            dir_groups.entry(parent.to_path_buf()).or_default().push(entry_path.to_path_buf());
                        }
                    }
                }
            }
        }

        self.update_progress(task_id, format!("Found {} directories with audio files", dir_groups.len())).await;

        // 2. Process each directory group as a book
        let total_groups = dir_groups.len();
        let mut processed_count = 0;

        // Pre-fetch all books (minimal) for the library to handle deletions and fast lookup
        // Returns: (id, path, hash, manual_corrected, match_pattern)
        let all_books_minimal = self.book_repo.find_all_minimal_by_library(library_id).await.unwrap_or_default();
        
        // Build lookup maps
        // Map: Path -> (id, manual_corrected, match_pattern)
        let mut book_path_map: HashMap<PathBuf, (String, i32, Option<String>)> = HashMap::new();
        let mut book_hash_map: HashMap<String, (String, i32, Option<String>)> = HashMap::new();
        
        for (id, path, hash, manual_corrected, match_pattern) in &all_books_minimal {
            book_path_map.insert(PathBuf::from(path), (id.clone(), *manual_corrected, match_pattern.clone()));
            book_hash_map.insert(hash.clone(), (id.clone(), *manual_corrected, match_pattern.clone()));
        }

        let manual_corrected_patterns: Vec<(String, String)> = all_books_minimal.iter()
            .filter(|(_, _, _, mc, mp)| *mc == 1 && mp.is_some())
            .map(|(id, _, _, _, mp)| (id.clone(), mp.clone().unwrap()))
            .collect();
            
        let mut found_book_ids: HashSet<String> = HashSet::new();

        for (dir, mut files) in dir_groups {
            // Check cancellation
            self.check_cancellation(task_id).await?;
            
            processed_count += 1;
            let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown");
            
            self.update_progress(task_id, format!("Processing ({}/{}): {}", processed_count, total_groups, dir_name)).await;

            // Sort files by filename using natural sort order (e.g. 1, 2, 10 instead of 1, 10, 2)
            files.sort_by(|a, b| natord::compare(a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()));

            // Optimization: Find existing book to avoid DB lookup
            let mut existing_info = book_path_map.get(&dir).cloned();
            
            // If not found by path, try hash (for moved books)
            if existing_info.is_none() {
                let book_hash = self.generate_book_hash(&dir);
                existing_info = book_hash_map.get(&book_hash).cloned();
            }

            match self.process_book_directory(library_id, &dir, &files, last_scanned, task_id, scraper_config, &manual_corrected_patterns, existing_info).await {
                Ok((book_id, status)) => {
                    scan_result.total_books += 1;
                    match status {
                        ScanStatus::Created => scan_result.books_created += 1,
                        ScanStatus::Updated => scan_result.books_updated += 1,
                        ScanStatus::Skipped => scan_result.books_skipped += 1,
                    }
                    found_book_ids.insert(book_id.clone());
                    debug!(book_id = %book_id, path = ?dir, status = ?status, "Processed book directory");
                }
                Err(e) => {
                    scan_result.failed_count += 1;
                    warn!(path = ?dir, error = %e, "Failed to process book directory");
                    scan_result.errors.push(format!(
                        "Failed to process {}: {}",
                        dir.display(),
                        e
                    ));
                }
            }

            // Periodic garbage collection to prevent memory buildup during large scans
            // Force GC after every directory to help debug memory issues with native plugins
            self.plugin_manager.garbage_collect_all().await;
        }

        // 3. Handle Deletions: Delete books that were not found in the scan and path does not exist
        for (id, path_str, _, _, _) in all_books_minimal {
            if !found_book_ids.contains(&id) {
                let path = Path::new(&path_str);
                if !path.exists() {
                    info!("Book path missing, deleting record: {}", path_str);
                    if let Err(e) = self.book_repo.delete(&id).await {
                        warn!("Failed to delete missing book {}: {}", id, e);
                    } else {
                        scan_result.books_deleted += 1;
                        if let Err(e) = self.chapter_repo.delete_by_book(&id).await {
                            warn!("Failed to delete chapters for missing book {}: {}", id, e);
                        }
                    }
                }
            }
        }

        // Final garbage collection after scan
        self.plugin_manager.garbage_collect_all().await;

        Ok(scan_result)
    }

    /// Process a directory containing audio files as a book
    pub(crate) async fn process_book_directory(
        &self,
        library_id: &str,
        dir: &Path,
        files: &[PathBuf],
        last_scanned: Option<chrono::DateTime<chrono::Utc>>,
        task_id: Option<&str>,
        scraper_config: &crate::db::models::ScraperConfig,
        manual_corrected_patterns: &[(String, String)],
        existing_info: Option<(String, i32, Option<String>)>,
    ) -> Result<(String, ScanStatus)> {
        // 0. Check New Chapter Protection (Manual Correction)
        for (book_id, pattern) in manual_corrected_patterns {
            if !pattern.is_empty() {
                let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if let Ok(re) = regex::Regex::new(pattern) {
                    if re.is_match(dir_name) {
                        info!("New Chapter Protection: Merging {} into existing book {}", dir_name, book_id);
                        let has_changes = self.process_chapters(book_id, files, last_scanned, task_id, scraper_config.prefer_audio_title, None).await?;
                        return Ok((book_id.clone(), if has_changes { ScanStatus::Updated } else { ScanStatus::Skipped }));
                    }
                }
            }
        }

        // 1. Check if Book Exists
        let mut existing_book_id = None;
        let mut is_manual_corrected = false;
        
        let book_hash = self.generate_book_hash(dir);

        if let Some((id, mc, _)) = existing_info {
            existing_book_id = Some(id);
            is_manual_corrected = mc == 1;
        } else if let Ok(Some(book)) = self.book_repo.find_by_hash(&book_hash).await {
            existing_book_id = Some(book.id.clone());
            is_manual_corrected = book.manual_corrected == 1;
        }

        // 2. Optimization: Skip metadata update if files haven't changed
        let max_mtime = files.iter()
            .filter_map(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()))
            .max();
        let max_mtime_utc = max_mtime.map(|t| chrono::DateTime::<chrono::Utc>::from(t));
        
        let mut skip_metadata_update = false;
        if let (Some(last_scan), Some(max_mt)) = (last_scanned, max_mtime_utc) {
            if max_mt <= last_scan && existing_book_id.is_some() {
                skip_metadata_update = true;
            }
        }

        if skip_metadata_update && existing_book_id.is_some() {
             let book_id = existing_book_id.unwrap();
             // Just process chapters (which also has skip logic)
             let has_changes = self.process_chapters(&book_id, files, last_scanned, task_id, scraper_config.prefer_audio_title, None).await?;
             return Ok((book_id, if has_changes { ScanStatus::Updated } else { ScanStatus::Skipped }));
        }

        // 3. Extract Metadata
        let (_quick_title, _) = self.text_cleaner.clean_chapter_title(dir.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown"), None);

        // Extended fields
        let mut subtitle: Option<String> = None;
        let mut published_year: Option<String> = None;
        let mut published_date: Option<String> = None;
        let mut publisher: Option<String> = None;
        let mut isbn: Option<String> = None;
        let mut asin: Option<String> = None;
        let mut language: Option<String> = None;
        let mut explicit: bool = false;
        let mut abridged: bool = false;
        let mut json_tags: Vec<String> = Vec::new();
        let mut json_series: Vec<String> = Vec::new();
        let mut json_chapters: Option<Vec<crate::core::metadata_writer::AudiobookshelfChapter>> = None;

        // 1. Extract from files/NFO
        let (mut title, mut author, mut narrator, mut description, mut tags, mut genre, mut cover_url, source) = self.extract_metadata(dir, files, scraper_config).await;

        // 2. Read metadata.json (Overrides NFO/Audio)
        if let Ok(Some(meta)) = crate::core::metadata_writer::read_metadata_json(dir) {
            if let Some(t) = meta.title { if !t.trim().is_empty() { title = t; } }
            if !meta.authors.is_empty() { author = Some(meta.authors[0].clone()); }
            if !meta.narrators.is_empty() { narrator = Some(meta.narrators[0].clone()); }
            if let Some(desc) = meta.description { if !desc.trim().is_empty() { description = Some(desc); } }
            if !meta.genres.is_empty() { genre = Some(meta.genres.join(",")); }
            if !meta.tags.is_empty() { 
                json_tags = meta.tags.clone();
                tags = Some(meta.tags.join(","));
            }
            if !meta.series.is_empty() { json_series = meta.series; }
            
            subtitle = meta.subtitle;
            published_year = meta.published_year;
            published_date = meta.published_date;
            publisher = meta.publisher;
            isbn = meta.isbn;
            asin = meta.asin;
            language = meta.language;
            if meta.explicit { explicit = true; }
            if meta.abridged { abridged = true; }
            if !meta.chapters.is_empty() { json_chapters = Some(meta.chapters); }
        }

        if author.is_none() { author = Some("Unknown".to_string()); }

        // 3. Apply Manual Correction or Existing Data
        if is_manual_corrected {
             if let Some(id) = &existing_book_id {
                if let Ok(Some(book)) = self.book_repo.find_by_id(id).await {
                    // Use existing values if present, otherwise fall back to extracted
                    title = book.title.unwrap_or(title);
                    if book.author.is_some() { author = book.author; }
                    if book.narrator.is_some() { narrator = book.narrator; }
                    if book.description.is_some() { description = book.description; }
                    if book.tags.is_some() { tags = book.tags; }
                    if book.genre.is_some() { genre = book.genre; }
                    if book.cover_url.is_some() { cover_url = book.cover_url; }
                    // theme_color will be recalculated if cover_url changed later
                }
             }
        } else {
            // Scraper logic (only if not manual corrected)
            if let Some(scraper) = &self.scraper_service {
                // ... (Existing Scraper Logic)
                // Check existing book by Title+Author to avoid dups if this is a new scan
                if existing_book_id.is_none() {
                     if let Some(ref a) = author {
                        if let Ok(Some(existing_book)) = self.book_repo.find_by_title_and_author(&title, a).await {
                            let existing_path = std::path::Path::new(&existing_book.path);
                            if existing_book.path != dir.to_string_lossy() && existing_path.exists() {
                                // Different book, ignore
                            } else {
                                existing_book_id = Some(existing_book.id.clone());
                                if existing_book.manual_corrected == 1 {
                                    // It was actually manual corrected!
                                    // Re-apply manual correction logic
                                    title = existing_book.title.unwrap_or(title);
                                    if existing_book.author.is_some() { author = existing_book.author; }
                                    if existing_book.narrator.is_some() { narrator = existing_book.narrator; }
                                    if existing_book.description.is_some() { description = existing_book.description; }
                                    if existing_book.tags.is_some() { tags = existing_book.tags; }
                                    if existing_book.genre.is_some() { genre = existing_book.genre; }
                                    if existing_book.cover_url.is_some() { cover_url = existing_book.cover_url; }
                                }
                            }
                        }
                    }
                }

                // If still not manual corrected (or we didn't find existing one), try scrape
                if existing_book_id.is_none() || (existing_book_id.is_some() && !is_manual_corrected) {
                    let needs_scrape = description.is_none() || published_year.is_none();
                    if needs_scrape {
                        self.update_progress(task_id, format!("Scraping metadata for: {}", title)).await;
                        match scraper.scrape_book_metadata(&title, scraper_config).await {
                            Ok(detail) => {
                                if !detail.intro.is_empty() && (source == MetadataSource::Fallback || description.is_none()) { description = Some(detail.intro); }
                                if !detail.tags.is_empty() && (source == MetadataSource::Fallback || tags.is_none()) { tags = Some(detail.tags.join(",")); }
                                
                                if let Some(g) = detail.genre {
                                    if !g.trim().is_empty() && (source == MetadataSource::Fallback || genre.is_none()) {
                                        genre = Some(g);
                                    }
                                }

                                if detail.cover_url.is_some() && (source == MetadataSource::Fallback || cover_url.is_none()) { cover_url = detail.cover_url; }
                                if detail.narrator.is_some() && (source == MetadataSource::Fallback || narrator.is_none()) { narrator = detail.narrator; }
                                if !detail.author.is_empty() && (source == MetadataSource::Fallback || author.as_deref() == Some("Unknown") || author.is_none()) { author = Some(detail.author); }
                                
                                if detail.subtitle.is_some() { subtitle = detail.subtitle; }
                                if detail.published_year.is_some() { published_year = detail.published_year; }
                                if detail.published_date.is_some() { published_date = detail.published_date; }
                                if detail.publisher.is_some() { publisher = detail.publisher; }
                                if detail.isbn.is_some() { isbn = detail.isbn; }
                                if detail.asin.is_some() { asin = detail.asin; }
                                if detail.language.is_some() { language = detail.language; }
                                if detail.explicit { explicit = true; }
                                if detail.abridged { abridged = true; }
                            },
                            Err(_) => {}
                        }
                    }
                }
            }
        }

        // Theme Color
        let mut theme_color = None;
        if let Some(ref url) = cover_url {
            let cover_path = if url.starts_with("http") { url.clone() } else {
                let p = Path::new(url);
                if p.exists() { url.clone() } else { dir.join(url).to_string_lossy().to_string() }
            };
            if let Ok(Some(color)) = crate::core::color::calculate_theme_color_with_client(&cover_path, &self.http_client).await {
                theme_color = Some(color);
            }
        }

        // 4. Create/Update Book
        let book_id = existing_book_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        
        let book = crate::db::models::Book {
            id: book_id.clone(),
            library_id: library_id.to_string(),
            title: Some(title.clone()),
            author: author.clone(),
            narrator: narrator.clone(),
            cover_url: cover_url.clone(),
            description: description.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            path: dir.to_string_lossy().to_string(),
            hash: book_hash.clone(),
            theme_color: theme_color.clone(),
            skip_intro: 0,
            skip_outro: 0,
            tags: tags.clone(),
            genre: genre.clone(),
            manual_corrected: if is_manual_corrected { 1 } else { 0 },
            match_pattern: None,
            chapter_regex: None,
        };

        let status = if let Ok(Some(existing)) = self.book_repo.find_by_id(&book_id).await {
            if existing.manual_corrected == 0 {
                self.book_repo.update(&book).await?;
                ScanStatus::Updated
            } else {
                ScanStatus::Skipped
            }
        } else {
            self.book_repo.create(&book).await?;
            ScanStatus::Created
        };

        // 5. Process Chapters
        let chapters_changed = self.process_chapters(&book_id, files, last_scanned, task_id, scraper_config.prefer_audio_title, json_chapters).await?;

        // 5.1 Process Series
        if !json_series.is_empty() {
            for series_title in json_series {
                if series_title.trim().is_empty() { continue; }
                
                // Find or create series
                let series = if let Some(s) = self.series_repo.find_by_title_and_library(&series_title, library_id).await? {
                    s
                } else {
                    let new_series = crate::db::models::Series {
                        id: Uuid::new_v4().to_string(),
                        library_id: library_id.to_string(),
                        title: series_title.clone(),
                        author: author.clone(),
                        narrator: narrator.clone(),
                        cover_url: cover_url.clone(),
                        description: None,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                    };
                    self.series_repo.create(&new_series).await?;
                    new_series
                };
                
                // Link book to series if not already linked
                let books = self.series_repo.find_books_by_series(&series.id).await?;
                if !books.iter().any(|(b, _)| b.id == book_id) {
                     let order = books.len() as i32 + 1;
                     self.series_repo.add_book(crate::db::models::SeriesBook {
                         series_id: series.id,
                         book_id: book_id.clone(),
                         book_order: order,
                     }).await?;
                }
            }
        }

        // 6. Write NFO/Metadata
        if scraper_config.nfo_writing_enabled {
             if let Ok(Some(book)) = self.book_repo.find_by_id(&book_id).await {
                let mut metadata = BookMetadata::new(book.title.clone().unwrap_or_default(), "ting-reader".to_string(), book.id.clone(), 0);
                metadata.author = book.author.clone();
                metadata.narrator = book.narrator.clone();
                metadata.intro = book.description.clone();
                metadata.cover_url = book.cover_url.clone();
                if let Some(tags_str) = &book.tags { metadata.tags.items = tags_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
                if let Err(e) = self.nfo_manager.write_book_nfo_to_dir(Path::new(&book.path), &metadata) {
                    warn!("Failed to write NFO: {}", e);
                }
            }
        }
        
        if scraper_config.metadata_writing_enabled {
            let chapters = self.chapter_repo.find_by_book(&book_id).await?;
            let mut sorted_chapters = chapters;
            sorted_chapters.sort_by(|a, b| a.chapter_index.unwrap_or(0).cmp(&b.chapter_index.unwrap_or(0)));
            let mut abs_chapters = Vec::new();
            let mut current_time = 0.0;
            for (idx, ch) in sorted_chapters.iter().enumerate() {
                let duration = ch.duration.unwrap_or(0) as f64;
                abs_chapters.push(crate::core::metadata_writer::AudiobookshelfChapter {
                    id: idx as u32,
                    start: current_time,
                    end: current_time + duration,
                    title: ch.title.clone().unwrap_or_default(),
                });
                current_time += duration;
            }
            let extended_meta = crate::core::metadata_writer::ExtendedMetadata {
                subtitle, published_year, published_date, publisher, isbn, asin, language,
                explicit, abridged, tags: json_tags,
            };
            
            // Get series for this book
            let series_list = self.series_repo.find_series_by_book(&book_id).await.unwrap_or_default();
            let series_titles: Vec<String> = series_list.into_iter().map(|s| s.title).collect();
            
            let metadata_json = crate::core::metadata_writer::AudiobookshelfMetadata::new(&book, abs_chapters, extended_meta, series_titles);
            if let Err(e) = crate::core::metadata_writer::write_metadata_json(dir, &metadata_json) {
                warn!("Failed to write metadata.json: {}", e);
            }
        }

        Ok((book_id, if chapters_changed { ScanStatus::Updated } else { status }))
    }

    async fn extract_metadata(&self, dir: &Path, files: &[PathBuf], scraper_config: &crate::db::models::ScraperConfig) -> (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, MetadataSource) {
        // Try NFO
        let nfo_path = dir.join("book.nfo");
        if let Ok(meta) = self.nfo_manager.read_book_nfo(&nfo_path) {
            return (
                meta.title,
                meta.author,
                meta.narrator,
                meta.intro,
                Some(meta.tags.items.join(",")),
                Some(meta.genre.items.join(",")),
                meta.cover_url,
                MetadataSource::Nfo
            );
        }

        // Try Audio Metadata from first file
        let mut title = String::new();
        let mut author = None;
        let mut narrator = None;
        let mut description = None;
        let tags = None;
        let mut genre = None;
        let mut cover_url_from_plugin = None;
        let mut source = MetadataSource::Fallback;

        if !files.is_empty() {
            let file_path = &files[0];
            
            // First try standard audio streamer (Symphonia)
            // But for encrypted files (like XM, NCM), standard streamer might fail or return garbage
            let ext = file_path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let is_standard = STANDARD_EXTENSIONS.contains(&ext.as_str());
            
            if is_standard {


                if let Ok(meta) = self.audio_streamer.read_metadata(file_path) {
                    if let Some(t) = meta.album {
                        if !t.trim().is_empty() {
                            title = t;
                            source = MetadataSource::FileMetadata;
                        }
                    }
                    
                    // Logic for Author/Narrator extraction
                    // Priority: AlbumArtist > Artist
                    if let Some(aa) = meta.album_artist {
                         if !aa.trim().is_empty() {
                             author = Some(aa);
                             source = MetadataSource::FileMetadata;
                         }
                    }
                    
                    if let Some(a) = meta.artist {
                        if !a.trim().is_empty() {
                            if author.is_none() {
                                author = Some(a.clone());
                                source = MetadataSource::FileMetadata;
                            } else if author.as_ref() != Some(&a) {
                                // If Author is already set (e.g. from AlbumArtist) and Artist is different,
                                // Artist is likely the Narrator or Contributing Artist.
                                narrator = Some(a);
                            }
                        }
                    }
                    
                    if let Some(c) = meta.composer {
                        if !c.trim().is_empty() {
                            if narrator.is_none() {
                                narrator = Some(c);
                            }
                        }
                    }
                    
                    if let Some(g) = meta.genre {
                        if !g.trim().is_empty() {
                            genre = Some(g);
                        }
                    }
                }
            }

            // If standard failed or it's an encrypted file, try plugins
            if title.is_empty() || !is_standard {
                // Try format plugins
                // Find a format plugin that supports this file
                let plugins = self.plugin_manager.find_plugins_by_type(PluginType::Format).await;
                for plugin in plugins {
                    // Check if plugin supports this extension
                    let supports_ext = plugin.supported_extensions.as_ref()
                        .map(|exts| exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
                        .unwrap_or(false);
                    
                    if !supports_ext {
                        continue;
                    }

                    // Call 'extract_metadata' on the plugin
                    let params = serde_json::json!({
                        "file_path": file_path.to_string_lossy()
                    });
                    
                    if let Ok(result) = self.plugin_manager.call_format(
                        &plugin.id, 
                        FormatMethod::ExtractMetadata, 
                        params
                    ).await {
                        // Parse result
                        if let Some(t) = result.get("album").and_then(|v| v.as_str()) {
                            if !t.trim().is_empty() {
                                title = t.to_string();
                                source = MetadataSource::FileMetadata;
                            }
                        }
                        
                        // Handle Author/Artist/Narrator
                        // Priority: album_artist (Author) > artist
                        if let Some(aa) = result.get("album_artist").and_then(|v| v.as_str()) {
                            if !aa.trim().is_empty() {
                                author = Some(aa.to_string());
                                source = MetadataSource::FileMetadata;
                            }
                        }
                        
                        // Handle Artist/Narrator based on Plugin ID
                        // For 'xm-format', the 'artist' field is actually the Narrator
                        if let Some(a) = result.get("artist").and_then(|v| v.as_str()) {
                            if !a.trim().is_empty() {
                                if author.is_none() {
                                    author = Some(a.to_string());
                                    source = MetadataSource::FileMetadata;
                                } else if author.as_ref().map(|s| s.as_str()) != Some(a) {
                                    // If author is set (e.g. from album_artist) and artist is different,
                                    // treat artist as narrator (if narrator not explicitly set later)
                                    if narrator.is_none() {
                                        narrator = Some(a.to_string());
                                    }
                                }
                            }
                        }
                        
                        // If plugin explicitly returns narrator (future proof)
                        if let Some(n) = result.get("narrator").and_then(|v| v.as_str()) {
                            if !n.trim().is_empty() {
                                narrator = Some(n.to_string());
                            }
                        }

                        if let Some(c) = result.get("cover_url").and_then(|v| v.as_str()) {
                            if !c.trim().is_empty() {
                                cover_url_from_plugin = Some(c.to_string());
                            }
                        }

                        if let Some(d) = result.get("description").and_then(|v| v.as_str()) {
                            if !d.trim().is_empty() {
                                description = Some(d.to_string());
                            }
                        }
                        
                        if let Some(g) = result.get("genre").and_then(|v| v.as_str()) {
                            if !g.trim().is_empty() {
                                genre = Some(g.to_string());
                            }
                        }
                        
                        // If we found metadata, break
                        if !title.is_empty() {
                            break;
                        }
                    }
                }
            }
        }

        // --- TITLE SELECTION LOGIC ---
        // Directory Name
        let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown Book");
        let (cleaned_dir_name, _) = self.text_cleaner.clean_chapter_title(dir_name, None);
        
        // Special handling for non-standard formats (Plugins)
        // User requested: "xm等需要格式支持插件的特殊格式所有规则不变"
        // If it's a special format and we successfully extracted a title, we should probably keep it
        // regardless of the `prefer_audio_title` setting, OR imply that for special formats we always prefer extracted title.
        // Let's assume if !is_standard and we have a title, we keep it.
        
        let mut is_special_format = false;
        if !files.is_empty() {
             let ext = files[0].extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
             if !STANDARD_EXTENSIONS.contains(&ext.as_str()) {
                 is_special_format = true;
             }
        }

        if is_special_format && !title.is_empty() {
            // Keep plugin title, do not fallback to directory
            // And clean it
            title = self.text_cleaner.clean_filename(&title);
        } else if scraper_config.prefer_audio_title && !title.is_empty() {
            // Use extracted ID3 title
            // Clean it just in case
            title = self.text_cleaner.clean_filename(&title);
        } else {
            // Prefer Directory Name (default behavior) or ID3 missing
            // Clean directory name
            title = cleaned_dir_name;
            source = MetadataSource::Fallback;
        }
        
        // --- TITLE SELECTION END ---

        // Fallback Author from "Author - Title" pattern
        if author.is_none() {
             let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
             if dir_name.contains(" - ") {
                 let parts: Vec<&str> = dir_name.split(" - ").collect();
                 if parts.len() >= 2 {
                     author = Some(parts[0].trim().to_string());
                 }
             }
        }

        // --- FIXED COVER EXTRACTION LOGIC START ---
        
        // 1. Try to find local cover image first (cover.jpg, etc.)
        let mut cover_url = self.find_cover_image(dir);
        
        // 2. If no local cover, check if plugin provided one (e.g. scraped URL)
        if cover_url.is_none() {
            cover_url = cover_url_from_plugin;
        }

        // 3. If still no cover, TRY TO EXTRACT FROM ID3 AND SAVE IT
        // This was missing: we need to actively extract and SAVE the cover to disk so `find_cover_image` or the frontend can use it.
        // We only do this if we have files.
        if cover_url.is_none() && !files.is_empty() {
             // Only try for MP3 files for now as we use id3 crate
             let first_file = &files[0];
             if let Some(ext) = first_file.extension() {
                 let ext_str = ext.to_string_lossy().to_lowercase();
                 if ext_str == "mp3" {
                     // This function extracts, saves to disk, and returns the relative path
                     if let Some(path) = self.extract_and_save_cover(first_file, dir) {
                         cover_url = Some(path);
                     }
                 }
             }
        }
        
        // --- FIXED COVER EXTRACTION LOGIC END ---

        (title, author, narrator, description, tags, genre, cover_url, source)
    }

    fn extract_and_save_cover(&self, audio_path: &Path, book_dir: &Path) -> Option<String> {
        if let Ok(tag) = id3::Tag::read_from_path(audio_path) {
            // Prefer CoverFront, otherwise take the first picture
            let picture = tag.pictures()
                .find(|p| p.picture_type == id3::frame::PictureType::CoverFront)
                .or_else(|| tag.pictures().next());

            if let Some(picture) = picture {
                // Determine extension from mime type
                let ext = match picture.mime_type.as_str() {
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/png" => "png",
                    "image/webp" => "webp",
                    "image/gif" => "gif",
                    _ => "jpg", // Default to jpg
                };
                
                let cover_filename = format!("cover.{}", ext);
                let cover_path = book_dir.join(&cover_filename);
                
                // Save to file
                // Force overwrite if needed? Or check existence?
                // If we are here, find_cover_image failed, so likely it doesn't exist.
                if let Err(e) = std::fs::write(&cover_path, &picture.data) {
                    warn!("Failed to save extracted cover to {:?}: {}", cover_path, e);
                    return None;
                }
                
                info!("Extracted cover from ID3 tag to {:?}", cover_path);
                
                // Return just the filename, find_cover_image will resolve it later or frontend uses relative
                // But wait, Book struct stores cover_url.
                // find_cover_image returns absolute path string.
                // We should return absolute path string here to match.
                // But wait, for local files, frontend might expect relative path if served statically?
                // Looking at find_cover_image: it returns `path.to_string_lossy().to_string()` which is absolute path.
                // So we should return absolute path.
                return Some(cover_path.to_string_lossy().replace('\\', "/"));
            }
        }
        None
    }

    pub(crate) async fn process_chapters(
        &self, 
        book_id: &str, 
        files: &[PathBuf], 
        last_scanned: Option<chrono::DateTime<chrono::Utc>>,
        task_id: Option<&str>,
        prefer_audio_title: bool,
        json_chapters: Option<Vec<crate::core::metadata_writer::AudiobookshelfChapter>>,
    ) -> Result<bool> {
        let mut has_changes = false;
        let total_files = files.len();
        
        // Use JSON chapters if available and count matches
        let use_json_chapters = if let Some(ref chapters) = json_chapters {
            if chapters.len() == total_files {
                info!("Using metadata.json chapters for book_id: {}", book_id);
                true
            } else {
                if !chapters.is_empty() {
                    warn!("metadata.json chapter count ({}) does not match file count ({}) for book {}. Ignoring JSON chapters.", chapters.len(), total_files, book_id);
                }
                false
            }
        } else {
            false
        };
        
        // Fetch book to check for regex rule
        let book = self.book_repo.find_by_id(book_id).await?
            .ok_or_else(|| TingError::NotFound("Book not found".to_string()))?;
            
        let chapter_regex = if let Some(pattern) = &book.chapter_regex {
            regex::Regex::new(pattern).ok()
        } else {
            None
        };

        // Pre-fetch existing chapters to support efficient incremental scanning
        // Map Path -> Chapter
        let existing_chapters = self.chapter_repo.find_by_book(book_id).await?;
        let mut chapter_map: HashMap<PathBuf, Chapter> = HashMap::new();
        for ch in existing_chapters {
            let p = PathBuf::from(&ch.path);
            chapter_map.insert(p, ch);
        }

        let mut main_counter = 0;
        let mut extra_counter = 0;

        for (index, file_path) in files.iter().enumerate() {
            if index % 5 == 0 {
                // Check cancellation and log progress
                self.check_cancellation(task_id).await?;
                self.update_progress(task_id, format!("Processing chapter {}/{}", index + 1, total_files)).await;
            }

            // Incremental Scan Logic
            // Check if file exists in DB
            let mut existing_chapter = chapter_map.get(file_path).cloned();
            
            // Check if file has changed
            let is_modified = if let Some(last_scan) = last_scanned {
                if let Ok(metadata) = std::fs::metadata(file_path) {
                    if let Ok(mtime) = metadata.modified() {
                        let mtime_utc: chrono::DateTime<chrono::Utc> = mtime.into();
                        mtime_utc > last_scan
                    } else {
                        true // Can't read mtime, force check
                    }
                } else {
                    true // Can't read metadata, force check
                }
            } else {
                true // No last scan, force check
            };

            // Optimization: If chapter exists and file is not modified, skip processing!
            if let Some(ref ch) = existing_chapter {
                if !is_modified {
                    // Update index if needed (e.g. reordering files), but skip hashing/metadata
                    // Also respect manual_corrected if we were to update anything else
                    
                    let is_extra_ch = ch.is_extra == 1;
                    let mut idx_from_counter = if is_extra_ch {
                         extra_counter += 1;
                         extra_counter
                    } else {
                         main_counter += 1;
                         main_counter
                    };
                    
                    // Apply regex if exists to override index
                    if let Some(re) = &chapter_regex {
                        if let Some(filename) = file_path.file_stem().and_then(|s| s.to_str()) {
                            if let Some(caps) = re.captures(filename) {
                                if let Some(m) = caps.get(1) {
                                    if let Ok(idx) = m.as_str().parse::<i32>() {
                                        idx_from_counter = idx;
                                    }
                                }
                            }
                        }
                    }

                    if ch.chapter_index != Some(idx_from_counter) {
                         // Only update index
                         // Check manual_corrected? 
                         // Usually index is structural, but if user manually ordered chapters, we might break it.
                         // But we sort files by name.
                         // If user manually corrected, we probably shouldn't touch index either?
                         // "manual_corrected" on chapter usually means Title correction.
                         // But let's respect it for index too if set.
                         if ch.manual_corrected == 0 {
                             let mut updated_ch = ch.clone();
                             updated_ch.chapter_index = Some(idx_from_counter);
                             self.chapter_repo.update(&updated_ch).await?;
                             has_changes = true;
                         }
                    }
                    continue;
                }
            }

            // If we are here, either it's a new file OR it's modified.
            
            // Calculate content-based hash
            let file_hash = self.calculate_file_hash(file_path)?;

            // Check if chapter exists by Hash (Global Deduplication)
            // But we must be careful: if we already found it by Path, we know it's that chapter.
            // If we found by Path, but Hash changed, it's an update.
            // If we didn't find by Path, we check Hash to see if it's a move/rename.
            
            if existing_chapter.is_none() {
                if let Ok(Some(ch)) = self.chapter_repo.find_by_hash(&file_hash).await {
                    // Found by hash (Rename/Move case)
                    // But we are processing a specific book_id here.
                    // If the found chapter belongs to another book, we might be stealing it?
                    // Or it's a duplicate file (e.g. same intro file in multiple books).
                    // If it's the same book, we treat it as the "existing chapter".
                    if ch.book_id == book_id {
                        existing_chapter = Some(ch);
                    }
                    // If different book, we create a new chapter record (duplicate content allowed across books)
                }
            }

            // Extract metadata
            let (_, extracted_title, _, _, _, duration) = self.extract_chapter_metadata(file_path).await;
            
            // Determine initial title based on preference
            let mut title = if use_json_chapters {
                // Priority 1: metadata.json (if count matches)
                if let Some(ref chapters) = json_chapters {
                    if index < chapters.len() {
                        chapters[index].title.clone()
                    } else {
                        // Should not happen if use_json_chapters is true
                        file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string()
                    }
                } else {
                    file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string()
                }
            } else if prefer_audio_title && !extracted_title.is_empty() {
                extracted_title
            } else {
                file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string()
            };

            // Check Regex for Title and Index
            let mut regex_idx = None;
            if let Some(re) = &chapter_regex {
                if let Some(filename) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(caps) = re.captures(filename) {
                         if let Some(m) = caps.get(1) {
                             if let Ok(idx) = m.as_str().parse::<i32>() {
                                 regex_idx = Some(idx);
                             }
                         }
                         // Only override title with regex if NOT using JSON chapters
                         if !use_json_chapters {
                             if let Some(m) = caps.get(2) {
                                 title = m.as_str().to_string(); // Update title from regex
                             }
                         }
                    }
                }
            }
            
            // Apply text cleaner to title
            let raw_title = title;
            
            let (final_title, is_extra) = if use_json_chapters {
                (raw_title, false)
            } else {
                self.text_cleaner.clean_chapter_title(&raw_title, book.title.as_deref())
            };

            // Calculate Index using counters
            let counter_idx = if is_extra {
                 extra_counter += 1;
                 extra_counter
            } else {
                 main_counter += 1;
                 main_counter
            };
            
            // Final Index
            let chapter_idx = regex_idx.unwrap_or_else(|| {
                 // If no regex rule, try to extract chapter number using TextCleaner heuristics
                 if chapter_regex.is_none() {
                     if let Some(filename) = file_path.file_stem().and_then(|s| s.to_str()) {
                         if let Some(idx) = self.text_cleaner.extract_chapter_number(filename) {
                             return idx;
                         }
                     }
                 }
                 counter_idx
            });

            if let Some(mut ch) = existing_chapter {
                // Update Existing
                // Check Lock
                if ch.manual_corrected == 0 {
                    ch.title = Some(final_title);
                    ch.chapter_index = Some(chapter_idx);
                    ch.is_extra = if is_extra { 1 } else { 0 };
                }
                // Always update duration/path/hash if file changed
                ch.path = file_path.to_string_lossy().to_string();
                ch.duration = Some(duration);
                ch.hash = Some(file_hash);
                ch.book_id = book_id.to_string();
                
                self.chapter_repo.update(&ch).await?;
                has_changes = true;
            } else {
                // Create New
                let chapter = Chapter {
                    id: Uuid::new_v4().to_string(),
                    book_id: book_id.to_string(),
                    title: Some(final_title),
                    path: file_path.to_string_lossy().to_string(),
                    duration: Some(duration),
                    chapter_index: Some(chapter_idx),
                    is_extra: if is_extra { 1 } else { 0 },
                    hash: Some(file_hash),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    manual_corrected: 0,
                };
                
                 match self.chapter_repo.create(&chapter).await {
                     Ok(_) => { has_changes = true; },
                     Err(e) => warn!("Failed to create chapter: {}", e),
                 }
            }
        }
        Ok(has_changes)
    }

    fn find_cover_image(&self, dir: &Path) -> Option<String> {
        let cover_names = ["cover.jpg", "cover.png", "cover.jpeg", "folder.jpg", "folder.png"];
        for name in cover_names {
            let path = dir.join(name);
            if path.exists() {
                // Return path with forward slashes for better JSON/URL compatibility
                return Some(path.to_string_lossy().replace('\\', "/"));
            }
        }
        if let Ok(mut entries) = std::fs::read_dir(dir) {
             while let Some(Ok(entry)) = entries.next() {
                 let path = entry.path();
                 if path.is_file() {
                     if let Some(ext) = path.extension() {
                         let ext_str = ext.to_string_lossy().to_lowercase();
                         if ["jpg", "jpeg", "png", "webp"].contains(&ext_str.as_str()) {
                             // Return path with forward slashes for better JSON/URL compatibility
                             return Some(path.to_string_lossy().replace('\\', "/"));
                         }
                     }
                 }
             }
        }
        None
    }

    fn generate_book_hash(&self, audiobook_dir: &Path) -> String {
        let path_str = audiobook_dir.to_string_lossy();
        let mut hasher = Sha256::new();
        hasher.update(path_str.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn calculate_file_hash(&self, path: &Path) -> Result<String> {
        let mut file = std::fs::File::open(path).map_err(|e| TingError::IoError(e))?;
        let metadata = file.metadata().map_err(|e| TingError::IoError(e))?;
        let len = metadata.len();
        
        let mut buffer = vec![0; 16384]; // 16KB
        let n = file.read(&mut buffer).map_err(|e| TingError::IoError(e))?;
        
        let mut hasher = Sha256::new();
        hasher.update(&buffer[..n]);
        hasher.update(len.to_le_bytes());
        // Also include filename to distinguish different chapters with same content/size (unlikely but possible)
        if let Some(name) = path.file_name() {
             hasher.update(name.to_string_lossy().as_bytes());
        }

        Ok(format!("{:x}", hasher.finalize()))
    }
}
