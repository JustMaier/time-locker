//! .7z.tlock File Format Implementation
//!
//! This module implements the .7z.tlock wrapper format that combines:
//! - An unencrypted header with metadata (readable without password)
//! - An encrypted 7z archive payload
//!
//! File Structure:
//! ```text
//! [.7z.tlock file]
//! +----------------------------------+
//! | HEADER (24 bytes, unencrypted)   |
//! |   Magic: "TLOCK01" (7 bytes)     |
//! |   Version: u8 (1 byte)           |
//! |   Metadata length: u32 LE (4 B)  |
//! |   Reserved: 12 bytes             |
//! +----------------------------------+
//! | METADATA (variable, unencrypted) |
//! |   JSON blob                      |
//! +----------------------------------+
//! | PAYLOAD (encrypted 7z archive)   |
//! +----------------------------------+
//! ```

use crate::archive::{create_encrypted_archive, extract_encrypted_archive};
use crate::error::{Result, TimeLockerError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// ============================================================================
// Constants
// ============================================================================

/// Magic bytes identifying a .7z.tlock file
pub const TLOCK_MAGIC: &[u8; 7] = b"TLOCK01";

/// Current format version
pub const TLOCK_VERSION: u8 = 1;

/// Fixed header size in bytes
pub const HEADER_SIZE: usize = 24;

/// Maximum allowed metadata size (1 MB should be more than enough)
pub const MAX_METADATA_SIZE: u32 = 1024 * 1024;

// ============================================================================
// Metadata Structure
// ============================================================================

/// Metadata stored in the unencrypted portion of a .7z.tlock file
///
/// This information is readable without the password, allowing the app
/// to display lock status, unlock time, and file info without decryption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlockMetadata {
    /// Whether the file is currently locked
    pub locked: bool,

    /// When the lock was created
    pub created: DateTime<Utc>,

    /// When the lock will unlock (time-lock expiry)
    pub unlocks: DateTime<Utc>,

    /// Human-readable duration string (e.g., "30d", "2026-07-01")
    pub duration: String,

    /// Original filename before archiving
    pub original_file: String,

    /// Drand round number used for time-lock encryption (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drand_round: Option<u64>,

    /// The encrypted symmetric key (AGE-encrypted with tlock)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_key: Option<String>,

    /// File size of the original content (before archiving)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_size: Option<u64>,

    /// Whether the original was a directory
    #[serde(default)]
    pub is_directory: bool,
}

impl TlockMetadata {
    /// Create new metadata for a time-locked file
    pub fn new(
        original_file: String,
        duration: String,
        unlocks: DateTime<Utc>,
        drand_round: Option<u64>,
        encrypted_key: Option<String>,
    ) -> Self {
        Self {
            locked: true,
            created: Utc::now(),
            unlocks,
            duration,
            original_file,
            drand_round,
            encrypted_key,
            original_size: None,
            is_directory: false,
        }
    }

    /// Check if the time lock has expired and file is unlockable
    pub fn is_unlockable(&self) -> bool {
        Utc::now() >= self.unlocks
    }

    /// Get time remaining until unlock
    pub fn time_until_unlock(&self) -> chrono::Duration {
        self.unlocks - Utc::now()
    }
}

// ============================================================================
// TlockArchive Implementation
// ============================================================================

/// Represents a .7z.tlock archive file
///
/// This struct provides methods for creating, reading metadata from,
/// and extracting .7z.tlock files.
#[derive(Debug)]
pub struct TlockArchive {
    /// Path to the .7z.tlock file
    pub path: PathBuf,

    /// Parsed metadata (if loaded)
    pub metadata: Option<TlockMetadata>,
}

impl TlockArchive {
    /// Create a new .7z.tlock file from a source file/directory
    ///
    /// # Arguments
    /// * `source_path` - Path to the file or directory to archive
    /// * `metadata` - Metadata to store in the unencrypted header
    /// * `password` - Password for the 7z encryption
    ///
    /// # Returns
    /// Path to the created .7z.tlock file
    ///
    /// # Process
    /// 1. Create encrypted 7z archive in temp location
    /// 2. Build header with magic bytes, version, metadata length
    /// 3. Serialize metadata as JSON
    /// 4. Write header + metadata + 7z payload to final .tlock file
    /// 5. Clean up temp 7z file
    pub fn create(
        source_path: &Path,
        metadata: TlockMetadata,
        password: &str,
    ) -> Result<PathBuf> {
        if !source_path.exists() {
            return Err(TimeLockerError::FileNotFound(
                source_path.display().to_string(),
            ));
        }

        eprintln!("[TlockArchive::create] Creating .7z.tlock from: {:?}", source_path);

        // Step 1: Create the encrypted 7z archive
        let temp_7z_path = create_encrypted_archive(source_path, password)?;

        // Step 2: Serialize metadata to JSON
        let metadata_json = serde_json::to_vec(&metadata)
            .map_err(|e| TimeLockerError::Parse(format!("Failed to serialize metadata: {}", e)))?;

        let metadata_len = metadata_json.len() as u32;
        if metadata_len > MAX_METADATA_SIZE {
            // Clean up temp file
            let _ = fs::remove_file(&temp_7z_path);
            return Err(TimeLockerError::Parse(format!(
                "Metadata too large: {} bytes (max: {})",
                metadata_len, MAX_METADATA_SIZE
            )));
        }

        // Step 3: Build the output path
        let tlock_path = source_path.with_extension("7z.tlock");

        eprintln!("[TlockArchive::create] Writing .7z.tlock to: {:?}", tlock_path);

        // Step 4: Write the .7z.tlock file
        let result = Self::write_tlock_file(&tlock_path, &metadata_json, &temp_7z_path);

        // Step 5: Clean up temp 7z file
        if let Err(e) = fs::remove_file(&temp_7z_path) {
            eprintln!("[TlockArchive::create] Warning: Failed to remove temp file: {}", e);
        }

        result?;

        eprintln!("[TlockArchive::create] Successfully created .7z.tlock file");
        Ok(tlock_path)
    }

    /// Write the complete .7z.tlock file
    fn write_tlock_file(
        tlock_path: &Path,
        metadata_json: &[u8],
        payload_path: &Path,
    ) -> Result<()> {
        let file = File::create(tlock_path)?;
        let mut writer = BufWriter::new(file);

        // Write header
        Self::write_header(&mut writer, metadata_json.len() as u32)?;

        // Write metadata
        writer.write_all(metadata_json)?;

        // Write payload (the encrypted 7z archive)
        let payload_file = File::open(payload_path)?;
        let mut payload_reader = BufReader::new(payload_file);
        std::io::copy(&mut payload_reader, &mut writer)?;

        writer.flush()?;
        Ok(())
    }

    /// Write the fixed-size header
    fn write_header<W: Write>(writer: &mut W, metadata_len: u32) -> Result<()> {
        // Magic bytes (7 bytes)
        writer.write_all(TLOCK_MAGIC)?;

        // Version (1 byte)
        writer.write_all(&[TLOCK_VERSION])?;

        // Metadata length (4 bytes, little-endian)
        writer.write_all(&metadata_len.to_le_bytes())?;

        // Reserved bytes (12 bytes)
        writer.write_all(&[0u8; 12])?;

        Ok(())
    }

    /// Read just the metadata from a .7z.tlock file (no password needed)
    ///
    /// # Arguments
    /// * `path` - Path to the .7z.tlock file
    ///
    /// # Returns
    /// A TlockArchive with loaded metadata
    ///
    /// # Errors
    /// - If the file doesn't exist
    /// - If the magic bytes don't match
    /// - If the version is unsupported
    /// - If metadata is corrupted
    pub fn read_metadata(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(TimeLockerError::FileNotFound(path.display().to_string()));
        }

        eprintln!("[TlockArchive::read_metadata] Reading: {:?}", path);

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read and validate header
        let (version, metadata_len) = Self::read_and_validate_header(&mut reader)?;

        eprintln!(
            "[TlockArchive::read_metadata] Version: {}, Metadata len: {}",
            version, metadata_len
        );

        // Read metadata JSON
        let mut metadata_bytes = vec![0u8; metadata_len as usize];
        reader.read_exact(&mut metadata_bytes).map_err(|e| {
            TimeLockerError::Parse(format!("Failed to read metadata: {}", e))
        })?;

        // Parse metadata
        let metadata: TlockMetadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| TimeLockerError::Parse(format!("Invalid metadata JSON: {}", e)))?;

        eprintln!(
            "[TlockArchive::read_metadata] Loaded metadata for: {}",
            metadata.original_file
        );

        Ok(Self {
            path: path.to_path_buf(),
            metadata: Some(metadata),
        })
    }

    /// Read and validate the file header
    ///
    /// Returns (version, metadata_length)
    fn read_and_validate_header<R: Read>(reader: &mut R) -> Result<(u8, u32)> {
        let mut header = [0u8; HEADER_SIZE];
        reader.read_exact(&mut header).map_err(|e| {
            TimeLockerError::Parse(format!("Failed to read header: {}", e))
        })?;

        // Validate magic bytes
        if &header[0..7] != TLOCK_MAGIC {
            return Err(TimeLockerError::Parse(
                "Invalid file: not a .7z.tlock file (bad magic bytes)".to_string(),
            ));
        }

        // Check version
        let version = header[7];
        if version > TLOCK_VERSION {
            return Err(TimeLockerError::Parse(format!(
                "Unsupported .7z.tlock version: {} (max supported: {})",
                version, TLOCK_VERSION
            )));
        }

        // Read metadata length
        let metadata_len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);

        // Sanity check
        if metadata_len > MAX_METADATA_SIZE {
            return Err(TimeLockerError::Parse(format!(
                "Metadata length exceeds maximum: {} > {}",
                metadata_len, MAX_METADATA_SIZE
            )));
        }

        Ok((version, metadata_len))
    }

    /// Extract the contents of a .7z.tlock file
    ///
    /// # Arguments
    /// * `path` - Path to the .7z.tlock file
    /// * `password` - Password for 7z decryption
    /// * `dest` - Destination directory for extracted files
    ///
    /// # Process
    /// 1. Validate header and skip metadata
    /// 2. Extract 7z payload to temp file
    /// 3. Extract temp 7z to destination
    /// 4. Clean up temp file
    pub fn extract(path: &Path, password: &str, dest: &Path) -> Result<()> {
        if !path.exists() {
            return Err(TimeLockerError::FileNotFound(path.display().to_string()));
        }

        eprintln!("[TlockArchive::extract] Extracting: {:?}", path);
        eprintln!("[TlockArchive::extract] Destination: {:?}", dest);

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read and validate header
        let (_version, metadata_len) = Self::read_and_validate_header(&mut reader)?;

        // Skip metadata section
        reader.seek(SeekFrom::Current(metadata_len as i64))?;

        // Create temp file for 7z payload
        let temp_dir = std::env::temp_dir();
        let temp_7z_path = temp_dir.join(format!(
            "tlock_extract_{}.7z",
            uuid::Uuid::new_v4()
        ));

        eprintln!("[TlockArchive::extract] Temp 7z: {:?}", temp_7z_path);

        // Extract payload to temp file
        {
            let temp_file = File::create(&temp_7z_path)?;
            let mut temp_writer = BufWriter::new(temp_file);
            std::io::copy(&mut reader, &mut temp_writer)?;
            temp_writer.flush()?;
        }

        // Extract the 7z archive
        let result = extract_encrypted_archive(&temp_7z_path, password, dest);

        // Clean up temp file
        if let Err(e) = fs::remove_file(&temp_7z_path) {
            eprintln!("[TlockArchive::extract] Warning: Failed to remove temp file: {}", e);
        }

        result?;

        eprintln!("[TlockArchive::extract] Extraction complete");
        Ok(())
    }

    /// Get the metadata (if loaded)
    pub fn get_metadata(&self) -> Option<&TlockMetadata> {
        self.metadata.as_ref()
    }

    /// Check if this archive is unlockable (time lock expired)
    pub fn is_unlockable(&self) -> bool {
        self.metadata
            .as_ref()
            .map(|m| m.is_unlockable())
            .unwrap_or(false)
    }

    /// Validate a file is a proper .7z.tlock file
    ///
    /// Performs quick validation without reading full metadata.
    pub fn validate(path: &Path) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Try to read header
        match Self::read_and_validate_header(&mut reader) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Get the payload offset (header size + metadata length)
    pub fn get_payload_offset(path: &Path) -> Result<u64> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let (_version, metadata_len) = Self::read_and_validate_header(&mut reader)?;

        Ok(HEADER_SIZE as u64 + metadata_len as u64)
    }

    /// Extract the 7z payload to a temporary file
    ///
    /// This is useful when you need the raw 7z archive for progress-enabled extraction.
    ///
    /// # Arguments
    /// * `path` - Path to the .7z.tlock file
    ///
    /// # Returns
    /// Path to the temporary 7z file (caller is responsible for cleanup)
    pub fn extract_payload_to_temp(path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            return Err(TimeLockerError::FileNotFound(path.display().to_string()));
        }

        eprintln!("[TlockArchive::extract_payload_to_temp] Extracting payload from: {:?}", path);

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read and validate header
        let (_version, metadata_len) = Self::read_and_validate_header(&mut reader)?;

        // Skip metadata section
        reader.seek(SeekFrom::Current(metadata_len as i64))?;

        // Create temp file for 7z payload
        let temp_dir = std::env::temp_dir();
        let temp_7z_path = temp_dir.join(format!(
            "tlock_extract_{}.7z",
            uuid::Uuid::new_v4()
        ));

        eprintln!("[TlockArchive::extract_payload_to_temp] Temp 7z: {:?}", temp_7z_path);

        // Extract payload to temp file
        {
            let temp_file = File::create(&temp_7z_path)?;
            let mut temp_writer = BufWriter::new(temp_file);
            std::io::copy(&mut reader, &mut temp_writer)?;
            temp_writer.flush()?;
        }

        Ok(temp_7z_path)
    }
}

// ============================================================================
// Scanning Functions
// ============================================================================

/// Scan a directory for .7z.tlock files
///
/// # Arguments
/// * `dir` - Directory to scan (recursively)
///
/// # Returns
/// Vector of TlockArchive with loaded metadata
pub fn scan_tlock_files(dir: &Path) -> Result<Vec<TlockArchive>> {
    use walkdir::WalkDir;

    let mut archives = Vec::new();

    if !dir.exists() || !dir.is_dir() {
        eprintln!("[scan_tlock_files] Directory does not exist or is not a dir: {:?}", dir);
        return Ok(archives);
    }

    eprintln!("[scan_tlock_files] Scanning directory: {:?}", dir);

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        // Check for .7z.tlock extension
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name.ends_with(".7z.tlock") {
                eprintln!("[scan_tlock_files] Found .7z.tlock file: {:?}", path);

                match TlockArchive::read_metadata(path) {
                    Ok(archive) => {
                        archives.push(archive);
                    }
                    Err(e) => {
                        eprintln!("[scan_tlock_files] Failed to read {:?}: {:?}", path, e);
                    }
                }
            }
        }
    }

    eprintln!("[scan_tlock_files] Found {} .7z.tlock files", archives.len());
    Ok(archives)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::fs;

    fn setup_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tlock_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_test_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_metadata_serialization() {
        let metadata = TlockMetadata::new(
            "test.txt".to_string(),
            "30d".to_string(),
            Utc::now() + Duration::days(30),
            Some(12345678),
            Some("encrypted_key_data".to_string()),
        );

        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: TlockMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.original_file, "test.txt");
        assert_eq!(parsed.duration, "30d");
        assert_eq!(parsed.drand_round, Some(12345678));
        assert!(parsed.locked);
    }

    #[test]
    fn test_metadata_is_unlockable() {
        // Future unlock time
        let future_metadata = TlockMetadata::new(
            "test.txt".to_string(),
            "30d".to_string(),
            Utc::now() + Duration::days(30),
            None,
            None,
        );
        assert!(!future_metadata.is_unlockable());

        // Past unlock time
        let past_metadata = TlockMetadata::new(
            "test.txt".to_string(),
            "0d".to_string(),
            Utc::now() - Duration::days(1),
            None,
            None,
        );
        assert!(past_metadata.is_unlockable());
    }

    #[test]
    fn test_create_and_read_metadata() -> Result<()> {
        let test_dir = setup_test_dir("create_read");

        // Create a test file
        let source_file = test_dir.join("secret.txt");
        fs::write(&source_file, b"This is secret content!")?;

        // Create metadata
        let metadata = TlockMetadata::new(
            "secret.txt".to_string(),
            "7d".to_string(),
            Utc::now() + Duration::days(7),
            Some(99999),
            Some("AGE_ENCRYPTED_KEY".to_string()),
        );

        // Create .7z.tlock file
        let password = "test_password_123";
        let tlock_path = TlockArchive::create(&source_file, metadata, password)?;

        assert!(tlock_path.exists());
        assert!(tlock_path.to_str().unwrap().ends_with(".7z.tlock"));

        // Read metadata (no password needed)
        let archive = TlockArchive::read_metadata(&tlock_path)?;
        let loaded_meta = archive.get_metadata().unwrap();

        assert_eq!(loaded_meta.original_file, "secret.txt");
        assert_eq!(loaded_meta.duration, "7d");
        assert_eq!(loaded_meta.drand_round, Some(99999));
        assert_eq!(loaded_meta.encrypted_key, Some("AGE_ENCRYPTED_KEY".to_string()));
        assert!(loaded_meta.locked);

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_create_and_extract() -> Result<()> {
        let test_dir = setup_test_dir("create_extract");

        // Create a test file
        let source_file = test_dir.join("document.txt");
        let content = b"Important document content for testing extraction!";
        fs::write(&source_file, content)?;

        // Create metadata
        let metadata = TlockMetadata::new(
            "document.txt".to_string(),
            "1d".to_string(),
            Utc::now() + Duration::days(1),
            None,
            None,
        );

        // Create .7z.tlock file
        let password = "extraction_test_pwd";
        let tlock_path = TlockArchive::create(&source_file, metadata, password)?;

        // Extract to new directory
        let extract_dir = test_dir.join("extracted");
        TlockArchive::extract(&tlock_path, password, &extract_dir)?;

        // Verify extracted file
        let extracted_file = extract_dir.join("document.txt");
        assert!(extracted_file.exists(), "Extracted file should exist");

        let extracted_content = fs::read(&extracted_file)?;
        assert_eq!(extracted_content, content);

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_wrong_password_fails() -> Result<()> {
        let test_dir = setup_test_dir("wrong_pwd");

        // Create a test file
        let source_file = test_dir.join("secret.txt");
        fs::write(&source_file, b"Secret!")?;

        // Create .7z.tlock file
        let metadata = TlockMetadata::new(
            "secret.txt".to_string(),
            "1d".to_string(),
            Utc::now() + Duration::days(1),
            None,
            None,
        );
        let tlock_path = TlockArchive::create(&source_file, metadata, "correct_password")?;

        // Try to extract with wrong password
        let extract_dir = test_dir.join("extracted");
        let result = TlockArchive::extract(&tlock_path, "wrong_password", &extract_dir);

        assert!(result.is_err());

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_validate_tlock_file() -> Result<()> {
        let test_dir = setup_test_dir("validate");

        // Create a valid .7z.tlock file
        let source_file = test_dir.join("test.txt");
        fs::write(&source_file, b"Test content")?;

        let metadata = TlockMetadata::new(
            "test.txt".to_string(),
            "1d".to_string(),
            Utc::now() + Duration::days(1),
            None,
            None,
        );
        let tlock_path = TlockArchive::create(&source_file, metadata, "password")?;

        // Valid file should validate
        assert!(TlockArchive::validate(&tlock_path)?);

        // Non-existent file should return false
        let fake_path = test_dir.join("nonexistent.7z.tlock");
        assert!(!TlockArchive::validate(&fake_path)?);

        // Create a fake file with wrong magic bytes
        let bad_file = test_dir.join("bad.7z.tlock");
        fs::write(&bad_file, b"NOT_A_TLOCK_FILE_AT_ALL")?;
        assert!(!TlockArchive::validate(&bad_file)?);

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_invalid_file_errors() -> Result<()> {
        let test_dir = setup_test_dir("invalid");

        // Test file too small
        let small_file = test_dir.join("small.7z.tlock");
        fs::write(&small_file, b"tiny")?;
        assert!(TlockArchive::read_metadata(&small_file).is_err());

        // Test wrong magic bytes
        let wrong_magic = test_dir.join("wrong_magic.7z.tlock");
        let mut bad_data = vec![0u8; 100];
        bad_data[0..7].copy_from_slice(b"BADMAGC");
        fs::write(&wrong_magic, &bad_data)?;
        let err = TlockArchive::read_metadata(&wrong_magic).unwrap_err();
        assert!(err.to_string().contains("bad magic bytes"));

        // Test unsupported version
        let bad_version = test_dir.join("bad_version.7z.tlock");
        let mut bad_ver_data = vec![0u8; 100];
        bad_ver_data[0..7].copy_from_slice(TLOCK_MAGIC);
        bad_ver_data[7] = 99; // Unsupported version
        fs::write(&bad_version, &bad_ver_data)?;
        let err = TlockArchive::read_metadata(&bad_version).unwrap_err();
        assert!(err.to_string().contains("Unsupported"));

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_scan_tlock_files() -> Result<()> {
        let test_dir = setup_test_dir("scan");

        // Create test files
        for i in 0..3 {
            let source = test_dir.join(format!("file{}.txt", i));
            fs::write(&source, format!("Content {}", i))?;

            let metadata = TlockMetadata::new(
                format!("file{}.txt", i),
                "1d".to_string(),
                Utc::now() + Duration::days(1),
                None,
                None,
            );
            TlockArchive::create(&source, metadata, "password")?;
        }

        // Create a subdirectory with another file
        let sub_dir = test_dir.join("subdir");
        fs::create_dir_all(&sub_dir)?;
        let sub_source = sub_dir.join("nested.txt");
        fs::write(&sub_source, b"Nested content")?;
        let nested_metadata = TlockMetadata::new(
            "nested.txt".to_string(),
            "1d".to_string(),
            Utc::now() + Duration::days(1),
            None,
            None,
        );
        TlockArchive::create(&sub_source, nested_metadata, "password")?;

        // Scan directory
        let archives = scan_tlock_files(&test_dir)?;

        assert_eq!(archives.len(), 4, "Should find 4 .7z.tlock files");

        cleanup_test_dir(&test_dir);
        Ok(())
    }

    #[test]
    fn test_header_constants() {
        // Verify header structure size
        assert_eq!(HEADER_SIZE, 24);
        assert_eq!(TLOCK_MAGIC.len(), 7);

        // 7 (magic) + 1 (version) + 4 (metadata len) + 12 (reserved) = 24
        assert_eq!(7 + 1 + 4 + 12, HEADER_SIZE);
    }

    #[test]
    fn test_directory_archiving() -> Result<()> {
        let test_dir = setup_test_dir("dir_archive");

        // Create a directory with multiple files
        let source_dir = test_dir.join("my_folder");
        fs::create_dir_all(&source_dir)?;
        fs::write(source_dir.join("file1.txt"), b"File 1 content")?;
        fs::write(source_dir.join("file2.txt"), b"File 2 content")?;

        let sub = source_dir.join("sub");
        fs::create_dir_all(&sub)?;
        fs::write(sub.join("nested.txt"), b"Nested file")?;

        // Create metadata for directory
        let mut metadata = TlockMetadata::new(
            "my_folder".to_string(),
            "1d".to_string(),
            Utc::now() + Duration::days(1),
            None,
            None,
        );
        metadata.is_directory = true;

        // Create .7z.tlock
        let password = "dir_test_pwd";
        let tlock_path = TlockArchive::create(&source_dir, metadata, password)?;

        assert!(tlock_path.exists());

        // Verify metadata
        let archive = TlockArchive::read_metadata(&tlock_path)?;
        let loaded = archive.get_metadata().unwrap();
        assert!(loaded.is_directory);
        assert_eq!(loaded.original_file, "my_folder");

        // Extract and verify
        let extract_dir = test_dir.join("extracted");
        TlockArchive::extract(&tlock_path, password, &extract_dir)?;

        // The folder structure should be preserved
        assert!(extract_dir.join("my_folder").exists() || extract_dir.join("file1.txt").exists());

        cleanup_test_dir(&test_dir);
        Ok(())
    }
}
