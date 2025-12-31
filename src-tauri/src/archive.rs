use crate::error::{Result, TimeLockerError};
use crate::progress::{ProgressEmitter, ProgressPhase, ProgressTracker};
use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
use sevenz_rust2::{decompress_with_password, ArchiveEntry, ArchiveWriter, Password};
use std::fs::{create_dir_all, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::Window;
use walkdir::WalkDir;

/// Create a password-protected 7z archive with encrypted headers (filenames hidden)
///
/// # Arguments
/// * `source_path` - Path to file or directory to archive
/// * `password` - Password for 7z encryption
///
/// # Returns
/// Path to the created 7z file
pub fn create_encrypted_archive(source_path: &Path, password: &str) -> Result<PathBuf> {
    if !source_path.exists() {
        return Err(TimeLockerError::FileNotFound(source_path.display().to_string()));
    }

    // Create output path with .7z extension
    let archive_path = source_path.with_extension("7z");

    eprintln!("[create_encrypted_archive] Creating 7z archive at: {:?}", archive_path);
    eprintln!("[create_encrypted_archive] Source: {:?}", source_path);
    eprintln!("[create_encrypted_archive] Password length: {}", password.len());

    // Use ArchiveWriter for header encryption support
    let mut writer = ArchiveWriter::create(&archive_path)
        .map_err(|e| TimeLockerError::Archive(format!("Failed to create archive writer: {}", e)))?;

    // Enable header encryption (hides filenames until password is entered)
    writer.set_encrypt_header(true);

    // Configure compression pipeline: AES encryption + LZMA2
    writer.set_content_methods(vec![
        AesEncoderOptions::new(password.into()).into(),
        Lzma2Options::from_level(6).into(),
    ]);

    // Add source to archive
    writer.push_source_path(source_path, |_| true)
        .map_err(|e| TimeLockerError::Archive(format!("Failed to add files: {}", e)))?;

    writer.finish()
        .map_err(|e| TimeLockerError::Archive(format!("Failed to finalize archive: {}", e)))?;

    eprintln!("[create_encrypted_archive] Archive created successfully (headers encrypted)");

    Ok(archive_path)
}

/// Create a password-protected 7z archive with progress tracking
///
/// This function uses ArchiveWriter to add files individually, allowing us to
/// track progress and report it to the frontend via Tauri events.
///
/// # Arguments
/// * `source_path` - Path to file or directory to archive
/// * `password` - Password for 7z encryption
/// * `window` - Tauri window handle for emitting progress events
/// * `tracker` - Optional shared progress tracker for cancellation support
///
/// # Returns
/// Path to the created 7z file
pub fn create_encrypted_archive_with_progress(
    source_path: &Path,
    password: &str,
    window: Window,
    tracker: Option<Arc<ProgressTracker>>,
) -> Result<PathBuf> {
    if !source_path.exists() {
        return Err(TimeLockerError::FileNotFound(
            source_path.display().to_string(),
        ));
    }

    // Create output path with .7z extension
    let archive_path = source_path.with_extension("7z");

    eprintln!(
        "[create_encrypted_archive_with_progress] Creating 7z archive at: {:?}",
        archive_path
    );
    eprintln!(
        "[create_encrypted_archive_with_progress] Source: {:?}",
        source_path
    );

    // Create or use provided tracker
    let tracker = tracker.unwrap_or_else(|| Arc::new(ProgressTracker::new()));
    let emitter = ProgressEmitter::new(window, Arc::clone(&tracker), "lock-progress");

    // Phase 1: Scanning - Calculate total size
    emitter.emit_progress_forced(None, ProgressPhase::Scanning);

    let (total_bytes, total_files) = crate::progress::calculate_total_size(source_path)
        .map_err(|e| TimeLockerError::Io(e))?;

    tracker.set_total(total_bytes, total_files);
    eprintln!(
        "[create_encrypted_archive_with_progress] Total: {} bytes, {} files",
        total_bytes, total_files
    );

    // Check for cancellation
    if tracker.is_cancelled() {
        return Err(TimeLockerError::Archive("Operation cancelled".to_string()));
    }

    // Phase 2: Compressing - Create archive with encryption
    emitter.emit_progress_forced(None, ProgressPhase::Compressing);

    let mut writer = ArchiveWriter::create(&archive_path)
        .map_err(|e| TimeLockerError::Archive(format!("Failed to create archive writer: {}", e)))?;

    // Enable header encryption (hides filenames)
    writer.set_encrypt_header(true);

    // Configure compression pipeline: AES encryption + LZMA2
    writer.set_content_methods(vec![
        AesEncoderOptions::new(password.into()).into(),
        Lzma2Options::from_level(6).into(), // Level 6 is a good balance
    ]);

    // Add files to the archive
    if source_path.is_file() {
        // Single file
        add_file_to_archive(&mut writer, source_path, source_path, &emitter, &tracker)?;
    } else if source_path.is_dir() {
        // Directory - walk and add all files
        for entry in WalkDir::new(source_path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            // Check for cancellation
            if tracker.is_cancelled() {
                // Clean up partial archive
                let _ = std::fs::remove_file(&archive_path);
                return Err(TimeLockerError::Archive("Operation cancelled".to_string()));
            }

            let path = entry.path();

            if path.is_file() {
                add_file_to_archive(&mut writer, path, source_path, &emitter, &tracker)?;
            } else if path.is_dir() && path != source_path {
                // Add directory entry (empty, just for structure)
                let relative_path = path
                    .strip_prefix(source_path)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace("\\", "/");

                let entry = ArchiveEntry::from_path(path, relative_path);
                writer
                    .push_archive_entry(entry, None::<std::io::Empty>)
                    .map_err(|e| {
                        TimeLockerError::Archive(format!("Failed to add directory entry: {}", e))
                    })?;
            }
        }
    }

    // Phase 3: Finalizing
    emitter.emit_progress_forced(None, ProgressPhase::Finalizing);

    writer.finish().map_err(|e| {
        TimeLockerError::Archive(format!("Failed to finalize archive: {}", e))
    })?;

    // Emit completion
    emitter.emit_complete();

    eprintln!("[create_encrypted_archive_with_progress] Archive created successfully");

    Ok(archive_path)
}

/// Helper function to add a single file to the archive with progress tracking
fn add_file_to_archive<W: std::io::Write + std::io::Seek>(
    writer: &mut ArchiveWriter<W>,
    file_path: &Path,
    base_path: &Path,
    emitter: &ProgressEmitter,
    tracker: &ProgressTracker,
) -> Result<()> {
    // Calculate relative path for archive entry name
    let relative_path = if file_path == base_path {
        // Single file - use just the filename
        file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    } else {
        // Directory member - use relative path
        file_path
            .strip_prefix(base_path)
            .unwrap_or(file_path)
            .to_string_lossy()
            .replace("\\", "/")
    };

    let file_name = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Emit progress for this file
    emitter.emit_progress(Some(file_name.clone()), ProgressPhase::Compressing);

    // Get file size for progress tracking
    let file_size = std::fs::metadata(file_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Create archive entry
    let entry = ArchiveEntry::from_path(file_path, relative_path);

    // Open file and add to archive
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    writer
        .push_archive_entry(entry, Some(reader))
        .map_err(|e| TimeLockerError::Archive(format!("Failed to add file '{}': {}", file_name, e)))?;

    // Update progress
    tracker.add_bytes(file_size);
    tracker.increment_files();

    // Force emit after each file for better feedback on large files
    emitter.emit_progress(Some(file_name), ProgressPhase::Compressing);

    Ok(())
}

/// Extract a password-protected 7z archive with progress tracking
///
/// # Arguments
/// * `archive_path` - Path to 7z file
/// * `password` - Password for decryption
/// * `dest` - Destination directory
/// * `window` - Tauri window handle for emitting progress events
/// * `tracker` - Optional shared progress tracker for cancellation support
pub fn extract_encrypted_archive_with_progress(
    archive_path: &Path,
    password: &str,
    dest: &Path,
    window: Window,
    tracker: Option<Arc<ProgressTracker>>,
) -> Result<()> {
    eprintln!(
        "[extract_encrypted_archive_with_progress] Extracting: {:?}",
        archive_path
    );
    eprintln!(
        "[extract_encrypted_archive_with_progress] Destination: {:?}",
        dest
    );

    let tracker = tracker.unwrap_or_else(|| Arc::new(ProgressTracker::new()));
    let emitter = ProgressEmitter::new(window, Arc::clone(&tracker), "unlock-progress");

    // Emit start of extraction
    emitter.emit_progress_forced(None, ProgressPhase::Extracting);

    // Create destination directory
    create_dir_all(dest)?;

    // Get archive size for progress estimation
    let archive_size = std::fs::metadata(archive_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // For extraction, we estimate based on archive size
    // (actual uncompressed size is not easily available without reading the archive)
    tracker.set_total(archive_size, 1);

    // Check for cancellation
    if tracker.is_cancelled() {
        return Err(TimeLockerError::Archive("Operation cancelled".to_string()));
    }

    // Open the archive file
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);

    // Extract using the helper function with password
    // Note: sevenz_rust2's decompress doesn't support progress callbacks,
    // so we emit progress at start and end only
    decompress_with_password(reader, dest, Password::from(password)).map_err(|e| {
        eprintln!(
            "[extract_encrypted_archive_with_progress] Extraction failed: {}",
            e
        );
        let err_str = e.to_string();
        if err_str.contains("password")
            || err_str.contains("Password")
            || err_str.contains("decrypt")
        {
            TimeLockerError::Decryption("Invalid password".to_string())
        } else {
            TimeLockerError::Archive(format!("Extraction failed: {}", e))
        }
    })?;

    // Update progress to complete
    tracker.set_bytes_written(archive_size);

    // Emit completion
    emitter.emit_complete();

    eprintln!("[extract_encrypted_archive_with_progress] Extraction complete");
    Ok(())
}

/// Extract a password-protected 7z archive
///
/// # Arguments
/// * `archive_path` - Path to 7z file
/// * `password` - Password for decryption
/// * `dest` - Destination directory
pub fn extract_encrypted_archive(archive_path: &Path, password: &str, dest: &Path) -> Result<()> {
    eprintln!("[extract_encrypted_archive] Extracting: {:?}", archive_path);
    eprintln!("[extract_encrypted_archive] Destination: {:?}", dest);

    // Create destination directory
    create_dir_all(dest)?;

    // Open the archive file
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);

    // Extract using the helper function with password
    decompress_with_password(reader, dest, Password::from(password))
        .map_err(|e| {
            eprintln!("[extract_encrypted_archive] Extraction failed: {}", e);
            let err_str = e.to_string();
            if err_str.contains("password") || err_str.contains("Password") || err_str.contains("decrypt") {
                TimeLockerError::Decryption("Invalid password".to_string())
            } else {
                TimeLockerError::Archive(format!("Extraction failed: {}", e))
            }
        })?;

    eprintln!("[extract_encrypted_archive] Extraction complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_create_and_extract_7z() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("test_7z_timelocker");
        let _ = fs::remove_dir_all(&temp_dir); // Clean up from previous runs
        create_dir_all(&temp_dir)?;

        // Create test file
        let test_file = temp_dir.join("test.txt");
        fs::write(&test_file, b"Hello, World!")?;

        // Create encrypted 7z
        let password = "test_password_123";
        let archive_path = create_encrypted_archive(&test_file, password)?;
        assert!(archive_path.exists());
        assert!(archive_path.extension().unwrap() == "7z");

        // Extract 7z
        let extract_dir = temp_dir.join("extracted");
        extract_encrypted_archive(&archive_path, password, &extract_dir)?;

        // The extracted file should be at extract_dir/test.txt
        let extracted_file = extract_dir.join("test.txt");
        assert!(extracted_file.exists(), "Extracted file should exist at {:?}", extracted_file);

        let content = fs::read_to_string(&extracted_file)?;
        assert_eq!(content, "Hello, World!");

        // Cleanup
        fs::remove_dir_all(&temp_dir)?;

        Ok(())
    }

    #[test]
    fn test_wrong_password_fails() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("test_7z_wrong_pwd_timelocker");
        let _ = fs::remove_dir_all(&temp_dir); // Clean up from previous runs
        create_dir_all(&temp_dir)?;

        // Create test file
        let test_file = temp_dir.join("secret.txt");
        fs::write(&test_file, b"Top Secret Data!")?;

        // Create encrypted 7z
        let correct_password = "correct_password";
        let archive_path = create_encrypted_archive(&test_file, correct_password)?;

        // Try to extract with wrong password - should fail
        let extract_dir = temp_dir.join("extracted_wrong");
        let wrong_password = "wrong_password";
        let result = extract_encrypted_archive(&archive_path, wrong_password, &extract_dir);

        assert!(result.is_err(), "Extraction with wrong password should fail!");
        eprintln!("Test passed: wrong password correctly rejected");

        // Cleanup
        fs::remove_dir_all(&temp_dir)?;

        Ok(())
    }

    #[test]
    fn test_header_encryption() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("test_7z_header_enc");
        let _ = fs::remove_dir_all(&temp_dir);
        create_dir_all(&temp_dir)?;

        // Create test file with recognizable name
        let test_file = temp_dir.join("VISIBLE_FILENAME.txt");
        fs::write(&test_file, b"Secret content here")?;

        // Create encrypted archive
        let password = "test_password";
        let archive_path = create_encrypted_archive(&test_file, password)?;

        // Read raw bytes and check for filename
        let data = fs::read(&archive_path)?;
        let data_str = String::from_utf8_lossy(&data);

        if data_str.contains("VISIBLE_FILENAME") {
            eprintln!("FAIL: Filename visible in raw archive bytes!");
            eprintln!("Header encryption is NOT working!");
            panic!("Header encryption failed - filename visible");
        } else {
            eprintln!("OK: Filename not visible in raw bytes - headers encrypted");
        }

        // Copy to vault for manual testing
        let _ = fs::copy(&archive_path, "E:/Vault/header_test.7z");
        eprintln!("Copied to E:/Vault/header_test.7z for manual verification");

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }

    #[test]
    fn test_header_encryption_large_file() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("test_7z_header_enc_large");
        let _ = fs::remove_dir_all(&temp_dir);
        create_dir_all(&temp_dir)?;

        // Create larger test file (100KB) with recognizable name
        let test_file = temp_dir.join("LARGE_SECRET_FILE.txt");
        let content: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        fs::write(&test_file, &content)?;

        eprintln!("Created test file: {} bytes", content.len());

        // Create encrypted archive
        let password = "test_password";
        let archive_path = create_encrypted_archive(&test_file, password)?;

        eprintln!("Archive created: {} bytes", fs::metadata(&archive_path)?.len());

        // Read raw bytes and check for filename (UTF-16 encoded)
        let data = fs::read(&archive_path)?;

        // Check for UTF-16 encoded filename "L.A.R.G.E._S.E.C.R.E.T"
        let mut found = false;
        for i in 0..data.len().saturating_sub(10) {
            // Look for 'L' followed by null byte (UTF-16 LE)
            if data[i] == b'L' && data.get(i + 1) == Some(&0)
                && data.get(i + 2) == Some(&b'A') && data.get(i + 3) == Some(&0)
                && data.get(i + 4) == Some(&b'R') && data.get(i + 5) == Some(&0)
            {
                found = true;
                eprintln!("FAIL: Found UTF-16 filename at offset 0x{:x}", i);
                break;
            }
        }

        // Also copy to vault for manual inspection
        let _ = fs::copy(&archive_path, "E:/Vault/header_test_large.7z");
        eprintln!("Copied to E:/Vault/header_test_large.7z");

        fs::remove_dir_all(&temp_dir)?;

        if found {
            panic!("Header encryption failed - filename visible in large file archive");
        } else {
            eprintln!("OK: Filename not visible in raw bytes - headers encrypted");
        }

        Ok(())
    }
}
