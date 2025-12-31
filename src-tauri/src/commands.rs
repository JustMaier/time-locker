use crate::keyfile::KeyFile;
use crate::progress::ProgressTracker;
use crate::tlock_format::{TlockArchive, TlockMetadata, scan_tlock_files};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::fs;
use tauri::{State, WebviewWindow};

/// Global state for tracking active operations (for cancellation support)
pub struct OperationState {
    /// Map of operation_id -> progress tracker
    pub active_operations: Mutex<HashMap<String, Arc<ProgressTracker>>>,
}

impl Default for OperationState {
    fn default() -> Self {
        Self {
            active_operations: Mutex::new(HashMap::new()),
        }
    }
}

/// Locked item representation for UI
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LockedItem {
    pub id: String,
    pub name: String,
    pub archive_path: String,
    /// Path to .key.md file (legacy format)
    pub key_path: String,
    /// Path to .7z.tlock file (new unified format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tlock_path: Option<String>,
    pub created_at: String,
    pub unlocks_at: String,
    pub is_unlockable: bool,
    pub original_file: Option<String>,
    /// Whether this is the legacy format (.key.md + .7z) vs new (.7z.tlock)
    #[serde(default)]
    pub is_legacy_format: bool,
    /// Whether the original file/folder was deleted after locking
    pub original_deleted: bool,
    /// Error message if deletion was requested but failed
    pub deletion_error: Option<String>,
    /// Path to the unlocked directory if it exists (indicates vault was previously unlocked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlocked_path: Option<String>,
}

/// Verify that a 7z archive exists and has valid structure
/// This performs a basic integrity check without needing the password
#[allow(dead_code)] // Kept for potential future legacy format support
fn verify_archive_exists_and_valid(archive_path: &std::path::Path) -> Result<(), String> {
    use std::io::Read;

    // Check file exists
    if !archive_path.exists() {
        return Err(format!("Archive file does not exist: {}", archive_path.display()));
    }

    // Check file has content (minimum size for a 7z archive)
    let metadata = fs::metadata(archive_path)
        .map_err(|e| format!("Failed to read archive metadata: {}", e))?;

    if metadata.len() < 32 {
        return Err("Archive file is too small to be valid".to_string());
    }

    // Verify 7z magic bytes (signature: 37 7A BC AF 27 1C)
    let mut file = fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open archive for verification: {}", e))?;

    let mut magic = [0u8; 6];
    file.read_exact(&mut magic)
        .map_err(|e| format!("Failed to read archive header: {}", e))?;

    const SEVEN_ZIP_MAGIC: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
    if magic != SEVEN_ZIP_MAGIC {
        return Err("Archive does not have valid 7z signature".to_string());
    }

    eprintln!("[verify_archive] Archive verified: {} ({} bytes)",
              archive_path.display(), metadata.len());
    Ok(())
}

/// Safely delete a file or directory
fn delete_source_safely(source_path: &std::path::Path) -> Result<(), String> {
    if !source_path.exists() {
        // Already deleted or never existed - not an error
        return Ok(());
    }

    if source_path.is_dir() {
        fs::remove_dir_all(source_path)
            .map_err(|e| format!("Failed to delete directory '{}': {}", source_path.display(), e))?;
        eprintln!("[delete_source] Deleted directory: {}", source_path.display());
    } else {
        fs::remove_file(source_path)
            .map_err(|e| format!("Failed to delete file '{}': {}", source_path.display(), e))?;
        eprintln!("[delete_source] Deleted file: {}", source_path.display());
    }

    Ok(())
}

/// Command to lock files with time-lock encryption
///
/// Creates a unified .7z.tlock file that contains:
/// - Unencrypted header with metadata (readable without password)
/// - Encrypted archive payload
#[tauri::command]
pub async fn lock_item(
    file_path: String,
    unlock_time: String,
    password: Option<String>,
    vault: Option<String>,
    delete_original: Option<bool>,
) -> Result<LockedItem, String> {
    use crate::crypto;
    use std::path::Path;

    let should_delete = delete_original.unwrap_or(false);

    eprintln!("[lock_item] Starting lock for: {}", file_path);
    eprintln!("[lock_item] Unlock time: {}", unlock_time);
    eprintln!("[lock_item] Vault: {:?}", vault);
    eprintln!("[lock_item] Delete original: {}", should_delete);

    // Validate unlock time is in the future
    let unlock_datetime = chrono::DateTime::parse_from_rfc3339(&unlock_time)
        .map_err(|e| format!("Invalid time format: {}", e))?;

    if unlock_datetime <= Utc::now() {
        return Err("Unlock time must be in the future".to_string());
    }

    let source_path = Path::new(&file_path);
    if !source_path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    // Store original path for potential deletion
    let original_source_path = source_path.to_path_buf();

    // Get original filename
    let original_filename = source_path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let is_directory = source_path.is_dir();

    // 1. Generate random password for the archive
    let archive_password = password.unwrap_or_else(|| crypto::generate_password(32));
    eprintln!("[lock_item] Generated password length: {}", archive_password.len());

    // 2. Encrypt the password with tlock (cryptographic time-lock)
    let unlock_utc = unlock_datetime.with_timezone(&Utc);
    let duration_str = unlock_datetime.format("%Y-%m-%d").to_string();

    let encrypted_password = crypto::encrypt_with_tlock(&archive_password, unlock_utc)
        .map_err(|e| format!("Failed to encrypt password with tlock: {}", e))?;
    eprintln!("[lock_item] Encrypted password with tlock");

    // 3. Get drand round number for metadata
    let drand_round = Some(crypto::datetime_to_round(unlock_utc));

    // 4. Create TlockMetadata
    let mut metadata = TlockMetadata::new(
        original_filename.clone(),
        duration_str,
        unlock_utc,
        drand_round,
        Some(encrypted_password),
    );
    metadata.is_directory = is_directory;

    // Get original size for metadata
    if let Ok((total_bytes, _)) = crate::progress::calculate_total_size(source_path) {
        metadata.original_size = Some(total_bytes);
    }

    // 5. Create the .7z.tlock file using TlockArchive
    let tlock_path = TlockArchive::create(source_path, metadata.clone(), &archive_password)
        .map_err(|e| format!("Failed to create .7z.tlock file: {}", e))?;

    eprintln!("[lock_item] Created .7z.tlock at: {:?}", tlock_path);

    // 6. Determine the vault directory and move file if needed
    let vault_dir = match vault {
        Some(ref v) if !v.is_empty() => PathBuf::from(v),
        _ => ensure_default_vault_exists()?,
    };

    let final_tlock_path = if vault_dir.exists() && vault_dir.is_dir() && tlock_path.parent() != Some(&vault_dir) {
        let tlock_filename = tlock_path.file_name().unwrap();
        let new_tlock_path = vault_dir.join(tlock_filename);
        fs::rename(&tlock_path, &new_tlock_path)
            .map_err(|e| format!("Failed to move .7z.tlock to vault: {}", e))?;
        eprintln!("[lock_item] Moved .7z.tlock to vault: {:?}", new_tlock_path);
        new_tlock_path
    } else {
        tlock_path
    };

    // 7. Handle original file deletion if requested
    let mut original_deleted = false;
    let mut deletion_error: Option<String> = None;

    if should_delete {
        eprintln!("[lock_item] Delete original requested, verifying .7z.tlock...");

        // Verify the .7z.tlock file was created successfully
        match TlockArchive::validate(&final_tlock_path) {
            Ok(true) => {
                // Safe to delete the original
                match delete_source_safely(&original_source_path) {
                    Ok(()) => {
                        original_deleted = true;
                        eprintln!("[lock_item] Original successfully deleted");
                    }
                    Err(e) => {
                        deletion_error = Some(e.clone());
                        eprintln!("[lock_item] Deletion failed: {}", e);
                    }
                }
            }
            Ok(false) => {
                deletion_error = Some(".7z.tlock file validation failed, refusing to delete original".to_string());
                eprintln!("[lock_item] Validation failed");
            }
            Err(e) => {
                deletion_error = Some(format!("Validation error: {}", e));
                eprintln!("[lock_item] Validation error: {}", e);
            }
        }
    }

    // Create LockedItem for response
    let tlock_path_str = final_tlock_path.display().to_string();
    let locked_item = LockedItem {
        id: generate_id_from_path(&tlock_path_str),
        name: original_filename,
        archive_path: tlock_path_str.clone(), // For backwards compat
        key_path: String::new(), // No separate key file in new format
        tlock_path: Some(tlock_path_str),
        created_at: metadata.created.to_rfc3339(),
        unlocks_at: metadata.unlocks.to_rfc3339(),
        is_unlockable: false,
        original_file: Some(file_path),
        is_legacy_format: false,
        original_deleted,
        deletion_error,
        unlocked_path: None, // Just locked, not unlocked yet
    };

    eprintln!("[lock_item] Lock complete: {:?}", locked_item);
    Ok(locked_item)
}

/// Command to lock files with time-lock encryption and progress tracking
///
/// Creates a unified .7z.tlock file with progress reporting.
/// Events are emitted on the "lock-progress" channel with ProgressPayload data.
#[tauri::command]
pub async fn lock_item_with_progress(
    window: WebviewWindow,
    state: State<'_, OperationState>,
    file_path: String,
    unlock_time: String,
    password: Option<String>,
    vault: Option<String>,
    delete_original: Option<bool>,
    operation_id: Option<String>,
) -> Result<LockedItem, String> {
    use crate::crypto;
    use crate::archive;
    use crate::tlock_format::TLOCK_MAGIC;
    use std::path::Path;
    use std::io::{Read, Write};

    let should_delete = delete_original.unwrap_or(false);
    let op_id = operation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    eprintln!("[lock_item_with_progress] Starting lock for: {}", file_path);
    eprintln!("[lock_item_with_progress] Operation ID: {}", op_id);
    eprintln!("[lock_item_with_progress] Unlock time: {}", unlock_time);
    eprintln!("[lock_item_with_progress] Vault: {:?}", vault);
    eprintln!("[lock_item_with_progress] Delete original: {}", should_delete);

    // Validate unlock time is in the future
    let unlock_datetime = chrono::DateTime::parse_from_rfc3339(&unlock_time)
        .map_err(|e| format!("Invalid time format: {}", e))?;

    if unlock_datetime <= Utc::now() {
        return Err("Unlock time must be in the future".to_string());
    }

    let source_path = Path::new(&file_path);
    if !source_path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    // Create progress tracker and register it for potential cancellation
    let tracker = Arc::new(ProgressTracker::new());
    {
        let mut ops = state.active_operations.lock().unwrap();
        ops.insert(op_id.clone(), Arc::clone(&tracker));
    }

    // Store original path for potential deletion
    let original_source_path = source_path.to_path_buf();

    // Get original filename and check if directory
    let original_filename = source_path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let is_directory = source_path.is_dir();

    // 1. Generate random password for the archive
    let archive_password = password.unwrap_or_else(|| crypto::generate_password(32));
    eprintln!("[lock_item_with_progress] Generated password length: {}", archive_password.len());

    // 2. Create encrypted 7z archive with progress tracking
    let archive_start = std::time::Instant::now();
    let archive_result = archive::create_encrypted_archive_with_progress(
        source_path,
        &archive_password,
        window.clone(),
        Some(Arc::clone(&tracker)),
    );

    // Check for cancellation
    if tracker.is_cancelled() {
        // Remove from active operations
        let mut ops = state.active_operations.lock().unwrap();
        ops.remove(&op_id);
        return Err("Operation cancelled by user".to_string());
    }

    let temp_archive_path = archive_result
        .map_err(|e| {
            // Remove from active operations on error
            let mut ops = state.active_operations.lock().unwrap();
            ops.remove(&op_id);
            format!("Failed to create encrypted archive: {}", e)
        })?;
    eprintln!("[lock_item_with_progress] Created temp 7z archive at: {:?} (took {:?})", temp_archive_path, archive_start.elapsed());

    // 3. Encrypt the password with tlock (cryptographic time-lock)
    let unlock_utc = unlock_datetime.with_timezone(&Utc);
    let duration_str = unlock_datetime.format("%Y-%m-%d").to_string();

    let tlock_start = std::time::Instant::now();
    let encrypted_password = crypto::encrypt_with_tlock(&archive_password, unlock_utc)
        .map_err(|e| format!("Failed to encrypt password with tlock: {}", e))?;
    eprintln!("[lock_item_with_progress] Encrypted password with tlock (took {:?})", tlock_start.elapsed());

    // 4. Get drand round and original size for metadata
    let drand_round = Some(crypto::datetime_to_round(unlock_utc));
    let original_size = crate::progress::calculate_total_size(source_path)
        .map(|(bytes, _)| bytes)
        .ok();

    // 5. Create TlockMetadata
    let mut metadata = TlockMetadata::new(
        original_filename.clone(),
        duration_str,
        unlock_utc,
        drand_round,
        Some(encrypted_password),
    );
    metadata.is_directory = is_directory;
    metadata.original_size = original_size;

    // 6. Serialize metadata to JSON
    let metadata_json = serde_json::to_vec(&metadata)
        .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
    let metadata_len = metadata_json.len() as u32;

    // 7. Read the 7z archive payload
    let mut archive_file = fs::File::open(&temp_archive_path)
        .map_err(|e| format!("Failed to open temp archive: {}", e))?;
    let mut archive_payload = Vec::new();
    archive_file.read_to_end(&mut archive_payload)
        .map_err(|e| format!("Failed to read temp archive: {}", e))?;

    // 8. Create the .7z.tlock file path
    let tlock_path = source_path.with_extension("7z.tlock");

    // 9. Write the .7z.tlock file
    let mut tlock_file = fs::File::create(&tlock_path)
        .map_err(|e| format!("Failed to create .7z.tlock file: {}", e))?;

    // Write header
    tlock_file.write_all(TLOCK_MAGIC)
        .map_err(|e| format!("Failed to write magic bytes: {}", e))?;
    tlock_file.write_all(&[1u8]) // Version
        .map_err(|e| format!("Failed to write version: {}", e))?;
    tlock_file.write_all(&metadata_len.to_le_bytes())
        .map_err(|e| format!("Failed to write metadata length: {}", e))?;
    tlock_file.write_all(&[0u8; 12]) // Reserved
        .map_err(|e| format!("Failed to write reserved bytes: {}", e))?;

    // Write metadata
    tlock_file.write_all(&metadata_json)
        .map_err(|e| format!("Failed to write metadata: {}", e))?;

    // Write payload
    tlock_file.write_all(&archive_payload)
        .map_err(|e| format!("Failed to write archive payload: {}", e))?;

    tlock_file.flush()
        .map_err(|e| format!("Failed to flush file: {}", e))?;

    eprintln!("[lock_item_with_progress] Created .7z.tlock at: {:?}", tlock_path);

    // 10. Clean up temp 7z file
    if let Err(e) = fs::remove_file(&temp_archive_path) {
        eprintln!("[lock_item_with_progress] Warning: Failed to remove temp file: {}", e);
    }

    // Remove from active operations
    {
        let mut ops = state.active_operations.lock().unwrap();
        ops.remove(&op_id);
    }

    // 11. Move to vault if needed
    let vault_dir = match vault {
        Some(ref v) if !v.is_empty() => PathBuf::from(v),
        _ => ensure_default_vault_exists()?,
    };

    let final_tlock_path = if vault_dir.exists() && vault_dir.is_dir() && tlock_path.parent() != Some(&vault_dir) {
        let tlock_filename = tlock_path.file_name().unwrap();
        let new_tlock_path = vault_dir.join(tlock_filename);
        fs::rename(&tlock_path, &new_tlock_path)
            .map_err(|e| format!("Failed to move .7z.tlock to vault: {}", e))?;
        eprintln!("[lock_item_with_progress] Moved .7z.tlock to vault: {:?}", new_tlock_path);
        new_tlock_path
    } else {
        tlock_path
    };

    // 12. Handle original file deletion if requested
    let mut original_deleted = false;
    let mut deletion_error: Option<String> = None;

    if should_delete {
        eprintln!("[lock_item_with_progress] Delete original requested, verifying .7z.tlock...");

        match TlockArchive::validate(&final_tlock_path) {
            Ok(true) => {
                match delete_source_safely(&original_source_path) {
                    Ok(()) => {
                        original_deleted = true;
                        eprintln!("[lock_item_with_progress] Original successfully deleted");
                    }
                    Err(e) => {
                        deletion_error = Some(e.clone());
                        eprintln!("[lock_item_with_progress] Deletion failed: {}", e);
                    }
                }
            }
            Ok(false) => {
                deletion_error = Some(".7z.tlock file validation failed".to_string());
            }
            Err(e) => {
                deletion_error = Some(format!("Validation error: {}", e));
            }
        }
    }

    // Create LockedItem for response
    let tlock_path_str = final_tlock_path.display().to_string();
    let locked_item = LockedItem {
        id: generate_id_from_path(&tlock_path_str),
        name: original_filename,
        archive_path: tlock_path_str.clone(),
        key_path: String::new(),
        tlock_path: Some(tlock_path_str),
        created_at: metadata.created.to_rfc3339(),
        unlocks_at: metadata.unlocks.to_rfc3339(),
        is_unlockable: false,
        original_file: Some(file_path),
        is_legacy_format: false,
        original_deleted,
        deletion_error,
        unlocked_path: None, // Just locked, not unlocked yet
    };

    eprintln!("[lock_item_with_progress] Lock complete: {:?}", locked_item);
    Ok(locked_item)
}

/// Command to cancel an active lock/unlock operation
#[tauri::command]
pub fn cancel_operation(
    state: State<'_, OperationState>,
    operation_id: String,
) -> Result<bool, String> {
    let ops = state.active_operations.lock().unwrap();
    if let Some(tracker) = ops.get(&operation_id) {
        tracker.cancel();
        eprintln!("[cancel_operation] Cancelled operation: {}", operation_id);
        Ok(true)
    } else {
        eprintln!("[cancel_operation] Operation not found: {}", operation_id);
        Ok(false)
    }
}

/// Command to unlock files with progress tracking
#[tauri::command]
pub async fn unlock_item_with_progress(
    window: WebviewWindow,
    state: State<'_, OperationState>,
    key_path: String,
    _password: Option<String>,
    operation_id: Option<String>,
) -> Result<String, String> {
    use crate::crypto;
    use crate::archive;
    use std::path::Path;

    let op_id = operation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let key_file_path = Path::new(&key_path);
    if !key_file_path.exists() {
        return Err(format!("Key file not found: {}", key_path));
    }

    // Create progress tracker
    let tracker = Arc::new(ProgressTracker::new());
    {
        let mut ops = state.active_operations.lock().unwrap();
        ops.insert(op_id.clone(), Arc::clone(&tracker));
    }

    // 1. Read and parse key file
    let content = fs::read_to_string(key_file_path)
        .map_err(|e| format!("Failed to read key file: {}", e))?;

    let keyfile = KeyFile::parse(&content)
        .map_err(|e| format!("Failed to parse key file: {}", e))?;

    // 2. Check if unlock time has passed
    if !keyfile.is_unlockable() {
        let remaining = keyfile.time_until_unlock();
        // Remove from active operations
        let mut ops = state.active_operations.lock().unwrap();
        ops.remove(&op_id);
        return Err(format!(
            "Time lock still active. Unlock in {} hours, {} minutes",
            remaining.num_hours(),
            remaining.num_minutes() % 60
        ));
    }

    // 3. Decrypt the AES-encrypted password
    let archive_password = crypto::decrypt_with_tlock(&keyfile.encrypted_body, keyfile.metadata.unlocks)
        .map_err(|e| format!("Failed to decrypt password: {}", e))?;

    // 4. Extract the 7z archive with the password
    let archive_path_str = keyfile.metadata.archive_path
        .ok_or_else(|| "Archive path not found in key file".to_string())?;

    let archive_path = Path::new(&archive_path_str);
    if !archive_path.exists() {
        // Remove from active operations
        let mut ops = state.active_operations.lock().unwrap();
        ops.remove(&op_id);
        return Err(format!("Archive file not found: {}", archive_path_str));
    }

    // Extract to same directory as archive
    let output_dir = archive_path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("unlocked_{}", keyfile.metadata.original_file));

    let result = archive::extract_encrypted_archive_with_progress(
        archive_path,
        &archive_password,
        &output_dir,
        window,
        Some(Arc::clone(&tracker)),
    );

    // Remove from active operations
    {
        let mut ops = state.active_operations.lock().unwrap();
        ops.remove(&op_id);
    }

    // Check for cancellation
    if tracker.is_cancelled() {
        return Err("Operation cancelled by user".to_string());
    }

    result.map_err(|e| format!("Failed to extract archive: {}", e))?;

    Ok(output_dir.display().to_string())
}

/// Command to unlock files
#[tauri::command]
pub async fn unlock_item(
    key_path: String,
    _password: Option<String>,
) -> Result<String, String> {
    use crate::crypto;
    use crate::archive;
    use std::path::Path;
    use std::fs;

    let key_file_path = Path::new(&key_path);
    if !key_file_path.exists() {
        return Err(format!("Key file not found: {}", key_path));
    }

    // 1. Read and parse key file
    let content = fs::read_to_string(key_file_path)
        .map_err(|e| format!("Failed to read key file: {}", e))?;

    let keyfile = KeyFile::parse(&content)
        .map_err(|e| format!("Failed to parse key file: {}", e))?;

    // 2. Check if unlock time has passed
    if !keyfile.is_unlockable() {
        let remaining = keyfile.time_until_unlock();
        return Err(format!(
            "Time lock still active. Unlock in {} hours, {} minutes",
            remaining.num_hours(),
            remaining.num_minutes() % 60
        ));
    }

    // 3. Decrypt the AES-encrypted password
    let archive_password = crypto::decrypt_with_tlock(&keyfile.encrypted_body, keyfile.metadata.unlocks)
        .map_err(|e| format!("Failed to decrypt password: {}", e))?;

    // 4. Extract the 7z archive with the password
    let archive_path_str = keyfile.metadata.archive_path
        .ok_or_else(|| "Archive path not found in key file".to_string())?;

    let archive_path = Path::new(&archive_path_str);
    if !archive_path.exists() {
        return Err(format!("Archive file not found: {}", archive_path_str));
    }

    // Extract to same directory as archive
    let output_dir = archive_path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("unlocked_{}", keyfile.metadata.original_file));

    archive::extract_encrypted_archive(archive_path, &archive_password, &output_dir)
        .map_err(|e| format!("Failed to extract archive: {}", e))?;

    Ok(output_dir.display().to_string())
}

/// Get all locked items (scan for both .7z.tlock and legacy .key.md files)
#[tauri::command]
pub async fn get_locked_items() -> Result<Vec<LockedItem>, String> {
    // Scan the default vault directory
    let default_vault = get_default_vault_path()?;

    if !default_vault.exists() {
        eprintln!("[get_locked_items] Default vault does not exist: {:?}", default_vault);
        return Ok(Vec::new());
    }

    let mut items: Vec<LockedItem> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Scan for new .7z.tlock files first
    if let Ok(tlock_archives) = scan_tlock_files(&default_vault) {
        for archive in tlock_archives {
            let path_str = archive.path.display().to_string();
            if !seen_paths.contains(&path_str) {
                seen_paths.insert(path_str);
                items.push(tlock_archive_to_locked_item(&archive));
            }
        }
    }

    // Also scan for legacy .key.md files
    if let Ok(key_files) = crate::keyfile::scan_directory(&default_vault) {
        for kf in key_files {
            if let Some(ref path) = kf.file_path {
                let path_str = path.display().to_string();
                if !seen_paths.contains(&path_str) {
                    seen_paths.insert(path_str);
                    items.push(keyfile_to_locked_item(&kf));
                }
            }
        }
    }

    Ok(items)
}

/// Scan for locked files in a directory (both .7z.tlock and legacy .key.md files)
#[tauri::command]
pub async fn scan_for_keys(directory: Option<String>) -> Result<Vec<LockedItem>, String> {
    let scan_dir = match directory {
        Some(d) => PathBuf::from(d),
        None => get_default_vault_path()?,
    };

    if !scan_dir.exists() {
        eprintln!("[scan_for_keys] Directory does not exist: {:?}", scan_dir);
        return Ok(Vec::new());
    }

    let mut items: Vec<LockedItem> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Scan for new .7z.tlock files first
    if let Ok(tlock_archives) = scan_tlock_files(&scan_dir) {
        for archive in tlock_archives {
            let path_str = archive.path.display().to_string();
            if !seen_paths.contains(&path_str) {
                seen_paths.insert(path_str);
                items.push(tlock_archive_to_locked_item(&archive));
            }
        }
    }

    // Also scan for legacy .key.md files
    if let Ok(key_files) = crate::keyfile::scan_directory(&scan_dir) {
        for kf in key_files {
            if let Some(ref path) = kf.file_path {
                let path_str = path.display().to_string();
                if !seen_paths.contains(&path_str) {
                    seen_paths.insert(path_str);
                    items.push(keyfile_to_locked_item(&kf));
                }
            }
        }
    }

    Ok(items)
}

/// Validate if the unlock time has been reached
#[tauri::command]
pub fn validate_unlock_time(unlock_time_str: String) -> Result<bool, String> {
    let unlock_time = chrono::DateTime::parse_from_rfc3339(&unlock_time_str)
        .map_err(|e| format!("Invalid time format: {}", e))?;

    let now = Utc::now();
    Ok(unlock_time.timestamp() <= now.timestamp())
}

/// Get the executable directory
fn get_exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| format!("Failed to get executable path: {}", e))?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Failed to get executable directory".to_string())
}

/// Get path to settings file (next to executable)
fn get_settings_path() -> Result<PathBuf, String> {
    Ok(get_exe_dir()?.join("timelocker-settings.json"))
}

/// Get the default vault path ({exe_dir}/vaults/)
fn get_default_vault_path() -> Result<PathBuf, String> {
    Ok(get_exe_dir()?.join("vaults"))
}

/// Ensure the default vault directory exists (creates it if needed)
fn ensure_default_vault_exists() -> Result<PathBuf, String> {
    let vault_path = get_default_vault_path()?;
    if !vault_path.exists() {
        fs::create_dir_all(&vault_path)
            .map_err(|e| format!("Failed to create default vault directory: {}", e))?;
        eprintln!("[ensure_default_vault_exists] Created default vault at: {:?}", vault_path);
    }
    Ok(vault_path)
}

/// Settings structure
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AppSettings {
    pub vaults: Vec<String>,
}

/// Complete application state returned to frontend
#[derive(Debug, Serialize, Deserialize)]
pub struct AppState {
    pub settings: AppSettings,
    pub locked_items: Vec<LockedItem>,
}

/// Get application settings from JSON file
#[tauri::command]
pub async fn get_settings() -> Result<AppSettings, String> {
    use std::fs;

    let settings_path = get_settings_path()?;

    if !settings_path.exists() {
        return Ok(AppSettings::default());
    }

    let content = fs::read_to_string(&settings_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: AppSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    Ok(settings)
}

/// Save application settings to JSON file
#[tauri::command]
pub async fn save_settings(settings: AppSettings) -> Result<(), String> {
    let settings_path = get_settings_path()?;

    let content = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    fs::write(&settings_path, content)
        .map_err(|e| format!("Failed to write settings file: {}", e))?;

    Ok(())
}

/// Get complete application state (settings + all locked items)
/// This is the single source of truth for the frontend
///
/// Scans for both:
/// - New format: .7z.tlock files (unified format)
/// - Legacy format: .key.md files (for backwards compatibility)
#[tauri::command]
pub async fn get_app_state() -> Result<AppState, String> {
    // Load settings
    let settings = get_settings_internal()?;

    let mut all_items: Vec<LockedItem> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Helper closure to scan a directory for both formats
    let scan_directory = |dir: &PathBuf, items: &mut Vec<LockedItem>, seen: &mut std::collections::HashSet<String>| {
        if !dir.exists() {
            return;
        }

        eprintln!("[get_app_state] Scanning directory: {:?}", dir);

        // Scan for new .7z.tlock files first (preferred format)
        if let Ok(tlock_archives) = scan_tlock_files(dir) {
            for archive in tlock_archives {
                let path_str = archive.path.display().to_string();
                if !seen.contains(&path_str) {
                    seen.insert(path_str.clone());
                    items.push(tlock_archive_to_locked_item(&archive));
                }
            }
        }

        // Also scan for legacy .key.md files (backwards compatibility)
        if let Ok(key_files) = crate::keyfile::scan_directory(dir) {
            for kf in key_files {
                if let Some(ref path) = kf.file_path {
                    let path_str = path.display().to_string();
                    // Skip if we already have this item (e.g., if both formats exist)
                    if !seen.contains(&path_str) {
                        // Also check if there's a .7z.tlock version of this file
                        let tlock_version = path.with_extension("7z.tlock");
                        let tlock_str = tlock_version.display().to_string();
                        if !seen.contains(&tlock_str) {
                            seen.insert(path_str.clone());
                            items.push(keyfile_to_locked_item(&kf));
                        }
                    }
                }
            }
        }
    };

    // Scan default vault directory ({exe_dir}/vaults/) if it exists
    if let Ok(default_vault) = get_default_vault_path() {
        scan_directory(&default_vault, &mut all_items, &mut seen_paths);
    }

    // Scan each user-added vault directory
    for vault in &settings.vaults {
        let vault_path = PathBuf::from(vault);
        // Skip if this is the default vault (already scanned)
        if let Ok(default_vault) = get_default_vault_path() {
            if vault_path == default_vault {
                continue;
            }
        }
        scan_directory(&vault_path, &mut all_items, &mut seen_paths);
    }

    eprintln!("[get_app_state] Total items found: {}", all_items.len());

    Ok(AppState {
        settings,
        locked_items: all_items,
    })
}

/// Internal helper to get settings without async
fn get_settings_internal() -> Result<AppSettings, String> {
    let settings_path = get_settings_path()?;

    if !settings_path.exists() {
        return Ok(AppSettings::default());
    }

    let content = fs::read_to_string(&settings_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: AppSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    Ok(settings)
}

/// Generate a deterministic ID from a file path
fn generate_id_from_path(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Check if an unlocked directory exists for a given vault file
fn find_unlocked_path(vault_path: &std::path::Path, original_file: &str) -> Option<String> {
    let parent = vault_path.parent()?;
    let unlocked_dir = parent.join(format!("unlocked_{}", original_file));

    if unlocked_dir.exists() {
        Some(unlocked_dir.display().to_string())
    } else {
        None
    }
}

/// Convert KeyFile to LockedItem for frontend (legacy format)
fn keyfile_to_locked_item(kf: &KeyFile) -> LockedItem {
    let now = Utc::now();
    let is_unlockable = kf.metadata.unlocks <= now;
    let key_path = kf.file_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();

    // Check if unlocked directory exists
    let unlocked_path = kf.file_path.as_ref()
        .and_then(|p| find_unlocked_path(p, &kf.metadata.original_file));

    LockedItem {
        id: generate_id_from_path(&key_path),
        name: kf.metadata.original_file.clone(),
        archive_path: kf.metadata.archive_path.clone().unwrap_or_default(),
        key_path,
        tlock_path: None, // Legacy format has no .7z.tlock file
        created_at: kf.metadata.created.to_rfc3339(),
        unlocks_at: kf.metadata.unlocks.to_rfc3339(),
        is_unlockable,
        original_file: Some(kf.metadata.original_file.clone()),
        is_legacy_format: true, // This is the legacy format
        original_deleted: false,
        deletion_error: None,
        unlocked_path,
    }
}

/// Convert TlockArchive to LockedItem for frontend (new unified format)
fn tlock_archive_to_locked_item(archive: &TlockArchive) -> LockedItem {
    let now = Utc::now();
    let tlock_path = archive.path.display().to_string();

    // Get metadata if available
    let (name, created_at, unlocks_at, is_unlockable, original_file_name) = match archive.get_metadata() {
        Some(meta) => (
            meta.original_file.clone(),
            meta.created.to_rfc3339(),
            meta.unlocks.to_rfc3339(),
            meta.is_unlockable(),
            meta.original_file.clone(),
        ),
        None => (
            archive.path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string(),
            now.to_rfc3339(),
            now.to_rfc3339(),
            false,
            archive.path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string(),
        ),
    };

    // Check if unlocked directory exists
    let unlocked_path = find_unlocked_path(&archive.path, &original_file_name);

    LockedItem {
        id: generate_id_from_path(&tlock_path),
        name,
        archive_path: tlock_path.clone(), // For backwards compat
        key_path: String::new(), // No separate key file in new format
        tlock_path: Some(tlock_path),
        created_at,
        unlocks_at,
        is_unlockable,
        original_file: Some(original_file_name),
        is_legacy_format: false, // This is the new unified format
        original_deleted: false,
        deletion_error: None,
        unlocked_path,
    }
}

// ============================================================================
// MIGRATION: .key.md + .7z -> .7z.tlock
// ============================================================================

/// Result of migration operation
#[derive(Debug, Serialize, Deserialize)]
pub struct MigrationResult {
    pub success: bool,
    pub tlock_path: String,
    pub message: String,
    /// Whether old files were deleted
    pub old_files_deleted: bool,
}

/// Response structure for tlock metadata (without the encrypted key)
#[derive(Debug, Serialize, Deserialize)]
pub struct TlockMetadataResponse {
    pub locked: bool,
    pub created: String,
    pub unlocks: String,
    pub duration: String,
    pub original_file: String,
    pub is_unlockable: bool,
    pub is_directory: bool,
    pub original_size: Option<u64>,
}

/// Migrate from old format (.key.md + .7z) to new unified .7z.tlock format
///
/// # Arguments
/// * `key_md_path` - Path to the .key.md file
/// * `delete_old_files` - Whether to delete the old .key.md and .7z files after migration
///
/// # Returns
/// MigrationResult with success status and the path to the new .7z.tlock file
#[tauri::command]
pub async fn migrate_to_tlock(
    key_md_path: String,
    delete_old_files: Option<bool>,
) -> Result<MigrationResult, String> {
    use crate::tlock_format::{TlockArchive, TlockMetadata, TLOCK_MAGIC};
    use std::io::{Read, Write};
    use std::path::Path;

    let delete_old = delete_old_files.unwrap_or(false);
    let key_path = Path::new(&key_md_path);

    eprintln!("[migrate_to_tlock] Starting migration for: {}", key_md_path);

    // 1. Validate key file exists
    if !key_path.exists() {
        return Err(format!("Key file not found: {}", key_md_path));
    }

    // Check if file has .key.md extension
    let file_name = key_path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !file_name.ends_with(".key.md") && !file_name.ends_with("-key.md") {
        return Err(format!("File does not appear to be a key file (expected .key.md): {}", key_md_path));
    }

    // 2. Parse the key file
    let key_content = fs::read_to_string(key_path)
        .map_err(|e| format!("Failed to read key file: {}", e))?;

    let keyfile = KeyFile::parse(&key_content)
        .map_err(|e| format!("Failed to parse key file: {}", e))?;

    eprintln!("[migrate_to_tlock] Parsed key file for: {}", keyfile.metadata.original_file);

    // 3. Locate the associated .7z archive
    let archive_path_str = keyfile.metadata.archive_path
        .as_ref()
        .ok_or_else(|| "Key file does not contain archive_path field".to_string())?;

    let archive_path = Path::new(archive_path_str);

    // If archive path is relative, resolve it relative to key file location
    let archive_path = if archive_path.is_absolute() {
        archive_path.to_path_buf()
    } else {
        key_path.parent()
            .unwrap_or(Path::new("."))
            .join(archive_path)
    };

    eprintln!("[migrate_to_tlock] Looking for archive at: {:?}", archive_path);

    if !archive_path.exists() {
        return Err(format!(
            "Associated archive not found: {}. The archive may have been moved or deleted.",
            archive_path.display()
        ));
    }

    // Verify it's a .7z file
    if archive_path.extension().and_then(|s| s.to_str()) != Some("7z") {
        return Err(format!("Archive file does not have .7z extension: {}", archive_path.display()));
    }

    // 4. Check if already migrated (a .7z.tlock already exists with same base name)
    let tlock_path = archive_path.with_extension("7z.tlock");
    if tlock_path.exists() {
        return Err(format!(
            "A .7z.tlock file already exists: {}. Migration may have already been performed.",
            tlock_path.display()
        ));
    }

    // 5. Create TlockMetadata from KeyFile
    let tlock_metadata = TlockMetadata {
        locked: keyfile.metadata.locked,
        created: keyfile.metadata.created,
        unlocks: keyfile.metadata.unlocks,
        duration: keyfile.metadata.duration.clone(),
        original_file: keyfile.metadata.original_file.clone(),
        drand_round: None, // Legacy files don't have drand round
        encrypted_key: Some(keyfile.encrypted_body.clone()),
        original_size: None,
        is_directory: false,
    };

    // 6. Serialize metadata to JSON
    let metadata_json = serde_json::to_vec(&tlock_metadata)
        .map_err(|e| format!("Failed to serialize metadata: {}", e))?;

    let metadata_len = metadata_json.len() as u32;

    eprintln!("[migrate_to_tlock] Metadata JSON size: {} bytes", metadata_len);

    // 7. Read the .7z archive payload
    let mut archive_file = fs::File::open(&archive_path)
        .map_err(|e| format!("Failed to open archive: {}", e))?;
    let mut archive_payload = Vec::new();
    archive_file.read_to_end(&mut archive_payload)
        .map_err(|e| format!("Failed to read archive: {}", e))?;

    eprintln!("[migrate_to_tlock] Archive payload size: {} bytes", archive_payload.len());

    // 8. Create the .7z.tlock file with wrapper format
    let mut tlock_file = fs::File::create(&tlock_path)
        .map_err(|e| format!("Failed to create .7z.tlock file: {}", e))?;

    // Write HEADER (unencrypted, fixed structure)
    // Using the format from tlock_format module:
    // - Magic bytes: "TLOCK01" (7 bytes)
    // - Version: u8 (1 byte)
    // - Metadata length: u32 LE (4 bytes)
    // - Reserved: 12 bytes
    tlock_file.write_all(TLOCK_MAGIC)
        .map_err(|e| format!("Failed to write magic bytes: {}", e))?;

    // Version byte
    tlock_file.write_all(&[1u8])
        .map_err(|e| format!("Failed to write version: {}", e))?;

    // Metadata length (4 bytes, little-endian)
    tlock_file.write_all(&metadata_len.to_le_bytes())
        .map_err(|e| format!("Failed to write metadata length: {}", e))?;

    // Reserved bytes (12 bytes)
    let reserved = [0u8; 12];
    tlock_file.write_all(&reserved)
        .map_err(|e| format!("Failed to write reserved bytes: {}", e))?;

    // Write METADATA (unencrypted JSON)
    tlock_file.write_all(&metadata_json)
        .map_err(|e| format!("Failed to write metadata: {}", e))?;

    // Write PAYLOAD (encrypted 7z archive)
    tlock_file.write_all(&archive_payload)
        .map_err(|e| format!("Failed to write archive payload: {}", e))?;

    tlock_file.flush()
        .map_err(|e| format!("Failed to flush file: {}", e))?;

    eprintln!("[migrate_to_tlock] Created .7z.tlock file at: {:?}", tlock_path);

    // 9. Verify the created file is valid
    match TlockArchive::validate(&tlock_path) {
        Ok(true) => {
            eprintln!("[migrate_to_tlock] Verified .7z.tlock file is valid");
        }
        Ok(false) => {
            // Clean up invalid file
            let _ = fs::remove_file(&tlock_path);
            return Err("Created .7z.tlock file failed validation".to_string());
        }
        Err(e) => {
            // Clean up on error
            let _ = fs::remove_file(&tlock_path);
            return Err(format!("Failed to validate created file: {}", e));
        }
    }

    // 10. Optionally delete old files
    let mut old_files_deleted = false;
    if delete_old {
        // Delete key file
        if let Err(e) = fs::remove_file(key_path) {
            eprintln!("[migrate_to_tlock] Warning: Failed to delete key file: {}", e);
        } else {
            eprintln!("[migrate_to_tlock] Deleted old key file: {:?}", key_path);
        }

        // Delete archive
        if let Err(e) = fs::remove_file(&archive_path) {
            eprintln!("[migrate_to_tlock] Warning: Failed to delete archive: {}", e);
        } else {
            eprintln!("[migrate_to_tlock] Deleted old archive: {:?}", archive_path);
        }

        old_files_deleted = true;
    }

    Ok(MigrationResult {
        success: true,
        tlock_path: tlock_path.display().to_string(),
        message: format!(
            "Successfully migrated '{}' to .7z.tlock format",
            keyfile.metadata.original_file
        ),
        old_files_deleted,
    })
}

/// Read metadata from a .7z.tlock file without extracting the archive
///
/// This allows inspecting locked files to show their metadata in the UI
#[tauri::command]
pub async fn read_tlock_metadata(tlock_path: String) -> Result<TlockMetadataResponse, String> {
    use crate::tlock_format::TlockArchive;
    use std::path::Path;

    let path = Path::new(&tlock_path);

    if !path.exists() {
        return Err(format!("File not found: {}", tlock_path));
    }

    // Check extension
    let file_name = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !file_name.ends_with(".7z.tlock") {
        return Err(format!("File does not appear to be a .7z.tlock file: {}", tlock_path));
    }

    let archive = TlockArchive::read_metadata(path)
        .map_err(|e| format!("Failed to read metadata: {}", e))?;

    let metadata = archive.get_metadata()
        .ok_or_else(|| "Metadata not found in archive".to_string())?;

    Ok(TlockMetadataResponse {
        locked: metadata.locked,
        created: metadata.created.to_rfc3339(),
        unlocks: metadata.unlocks.to_rfc3339(),
        duration: metadata.duration.clone(),
        original_file: metadata.original_file.clone(),
        is_unlockable: metadata.is_unlockable(),
        is_directory: metadata.is_directory,
        original_size: metadata.original_size,
    })
}

/// Check if a file is a valid .7z.tlock file
#[tauri::command]
pub fn is_tlock_file(file_path: String) -> Result<bool, String> {
    use crate::tlock_format::TlockArchive;
    use std::path::Path;

    let path = Path::new(&file_path);

    if !path.exists() {
        return Ok(false);
    }

    // Quick check by extension
    let file_name = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !file_name.ends_with(".7z.tlock") {
        return Ok(false);
    }

    // Verify using the tlock_format module
    TlockArchive::validate(path)
        .map_err(|e| format!("Failed to validate file: {}", e))
}

/// Check if a file is a legacy .key.md file that can be migrated
#[tauri::command]
pub fn is_legacy_key_file(file_path: String) -> Result<bool, String> {
    use std::path::Path;

    let path = Path::new(&file_path);

    if !path.exists() {
        return Ok(false);
    }

    let file_name = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Check extension pattern
    if !file_name.ends_with(".key.md") && !file_name.ends_with("-key.md") {
        return Ok(false);
    }

    // Try to parse as KeyFile to verify it's valid
    match fs::read_to_string(path) {
        Ok(content) => {
            match KeyFile::parse(&content) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        Err(_) => Ok(false),
    }
}

/// Open a path in the system file explorer (cross-platform)
#[tauri::command]
pub fn open_in_explorer(path: String) -> Result<(), String> {
    use std::process::Command;

    let path = std::path::Path::new(&path);

    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    eprintln!("[open_in_explorer] Opening: {:?}", path);

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open explorer: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open Finder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }

    Ok(())
}

/// Unlock a .7z.tlock file and extract its contents
///
/// # Arguments
/// * `tlock_path` - Path to the .7z.tlock file
/// * `output_dir` - Optional output directory (defaults to same directory as tlock file)
///
/// # Returns
/// Path to the extracted contents
#[tauri::command]
pub async fn unlock_tlock_file(
    window: WebviewWindow,
    tlock_path: String,
    output_dir: Option<String>,
) -> Result<String, String> {
    use crate::crypto;
    use crate::archive;
    use crate::tlock_format::TlockArchive;
    use crate::progress::{ProgressTracker, ProgressEmitter, ProgressPhase};
    use std::path::Path;

    let path = Path::new(&tlock_path);

    if !path.exists() {
        return Err(format!("File not found: {}", tlock_path));
    }

    eprintln!("[unlock_tlock_file] Starting unlock for: {}", tlock_path);

    // Create progress tracker for the unlock operation
    let tracker = Arc::new(ProgressTracker::new());
    let emitter = ProgressEmitter::new(window.clone(), Arc::clone(&tracker), "unlock-progress");

    // Emit decrypting phase
    emitter.emit_progress_forced(None, ProgressPhase::Encrypting);

    // 1. Read metadata from the .7z.tlock file
    let archive = TlockArchive::read_metadata(path)
        .map_err(|e| format!("Failed to read tlock file: {}", e))?;

    let metadata = archive.get_metadata()
        .ok_or_else(|| "Metadata not found in archive".to_string())?;

    eprintln!("[unlock_tlock_file] Parsed metadata for: {}", metadata.original_file);

    // 2. Check if unlock time has passed
    if !metadata.is_unlockable() {
        let remaining = metadata.time_until_unlock();
        return Err(format!(
            "Time lock still active. Unlock in {} hours, {} minutes",
            remaining.num_hours(),
            remaining.num_minutes() % 60
        ));
    }

    // 3. Decrypt the encrypted key to get the archive password
    let encrypted_key = metadata.encrypted_key.as_ref()
        .ok_or_else(|| "No encrypted key found in metadata".to_string())?;

    let archive_password = crypto::decrypt_with_tlock(encrypted_key, metadata.unlocks)
        .map_err(|e| format!("Failed to decrypt key: {}", e))?;

    eprintln!("[unlock_tlock_file] Decrypted archive password");

    // 4. Determine output directory
    let output_path = match output_dir {
        Some(dir) => PathBuf::from(dir),
        None => path.parent()
            .unwrap_or(Path::new("."))
            .join(format!("unlocked_{}", metadata.original_file)),
    };

    eprintln!("[unlock_tlock_file] Extracting to: {:?}", output_path);

    // 5. Extract the archive using progress-aware extraction
    // First, extract the 7z payload to a temp location then extract it
    let temp_archive = TlockArchive::extract_payload_to_temp(path)
        .map_err(|e| format!("Failed to extract archive payload: {}", e))?;

    // Use progress-enabled extraction
    archive::extract_encrypted_archive_with_progress(
        &temp_archive,
        &archive_password,
        &output_path,
        window,
        Some(tracker),
    ).map_err(|e| format!("Failed to extract archive: {}", e))?;

    // Clean up temp archive
    if let Err(e) = std::fs::remove_file(&temp_archive) {
        eprintln!("[unlock_tlock_file] Warning: Failed to remove temp file: {}", e);
    }

    eprintln!("[unlock_tlock_file] Extraction complete");

    Ok(output_path.display().to_string())
}
