//! Progress tracking module for compression/decompression operations
//!
//! This module provides utilities for tracking and reporting progress during
//! archive operations, including event emission to the Tauri frontend.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Window;

/// Progress update payload sent to the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressPayload {
    /// Percentage complete (0-100), None if total is unknown
    pub percentage: Option<f64>,
    /// Number of bytes processed so far
    pub bytes_written: u64,
    /// Total bytes to process, None if unknown
    pub total_bytes: Option<u64>,
    /// Estimated time remaining in seconds, None if cannot be calculated
    pub eta_seconds: Option<f64>,
    /// Current file being processed
    pub current_file: Option<String>,
    /// Number of files processed
    pub files_processed: u32,
    /// Total number of files, None if unknown
    pub total_files: Option<u32>,
    /// Current operation phase
    pub phase: ProgressPhase,
}

/// Phase of the operation for UI feedback
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    /// Scanning source files/directories
    Scanning,
    /// Compressing files
    Compressing,
    /// Encrypting the archive
    Encrypting,
    /// Finalizing the archive
    Finalizing,
    /// Operation complete
    Complete,
    /// Extracting files
    Extracting,
}

/// Thread-safe progress tracker that can be shared across operations
#[derive(Debug)]
pub struct ProgressTracker {
    /// Total bytes to process (if known)
    total_bytes: AtomicU64,
    /// Bytes processed so far
    bytes_written: AtomicU64,
    /// Total files to process (if known)
    total_files: AtomicU64,
    /// Files processed so far
    files_processed: AtomicU64,
    /// Whether total size is known
    total_known: AtomicBool,
    /// Cancellation flag
    cancelled: AtomicBool,
    /// Start time for ETA calculation
    start_time: Instant,
    /// Last emission time for throttling
    last_emit: std::sync::Mutex<Instant>,
    /// Minimum interval between emissions (milliseconds)
    throttle_ms: u64,
}

impl ProgressTracker {
    /// Create a new progress tracker with unknown total
    pub fn new() -> Self {
        Self {
            total_bytes: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            total_files: AtomicU64::new(0),
            files_processed: AtomicU64::new(0),
            total_known: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            start_time: Instant::now(),
            last_emit: std::sync::Mutex::new(Instant::now()),
            throttle_ms: 100, // Default: emit at most every 100ms
        }
    }

    /// Create a new progress tracker with known total bytes
    pub fn with_total(total_bytes: u64, total_files: u32) -> Self {
        let tracker = Self::new();
        tracker.set_total(total_bytes, total_files);
        tracker
    }

    /// Set the total bytes and files (can be called after scanning)
    pub fn set_total(&self, total_bytes: u64, total_files: u32) {
        self.total_bytes.store(total_bytes, Ordering::SeqCst);
        self.total_files.store(total_files as u64, Ordering::SeqCst);
        self.total_known.store(true, Ordering::SeqCst);
    }

    /// Add bytes to the processed count
    pub fn add_bytes(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::SeqCst);
    }

    /// Set the total bytes written (for when we know exact amount)
    pub fn set_bytes_written(&self, bytes: u64) {
        self.bytes_written.store(bytes, Ordering::SeqCst);
    }

    /// Increment the file counter
    pub fn increment_files(&self) {
        self.files_processed.fetch_add(1, Ordering::SeqCst);
    }

    /// Check if the operation has been cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Request cancellation of the operation
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Get current progress as percentage (0.0 - 100.0)
    pub fn percentage(&self) -> Option<f64> {
        if !self.total_known.load(Ordering::SeqCst) {
            return None;
        }
        let total = self.total_bytes.load(Ordering::SeqCst);
        if total == 0 {
            return Some(100.0);
        }
        let written = self.bytes_written.load(Ordering::SeqCst);
        Some((written as f64 / total as f64) * 100.0)
    }

    /// Calculate estimated time remaining in seconds
    pub fn eta_seconds(&self) -> Option<f64> {
        let percentage = self.percentage()?;
        if percentage <= 0.0 {
            return None;
        }
        if percentage >= 100.0 {
            return Some(0.0);
        }

        let elapsed = self.start_time.elapsed().as_secs_f64();
        let total_estimated = elapsed / (percentage / 100.0);
        let remaining = total_estimated - elapsed;

        if remaining.is_finite() && remaining >= 0.0 {
            Some(remaining)
        } else {
            None
        }
    }

    /// Check if enough time has passed since last emission (for throttling)
    pub fn should_emit(&self) -> bool {
        let mut last = self.last_emit.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(*last) >= Duration::from_millis(self.throttle_ms) {
            *last = now;
            true
        } else {
            false
        }
    }

    /// Force the next should_emit() to return true
    pub fn force_next_emit(&self) {
        let mut last = self.last_emit.lock().unwrap();
        *last = Instant::now() - Duration::from_millis(self.throttle_ms + 1);
    }

    /// Build a progress payload for the current state
    pub fn build_payload(&self, current_file: Option<String>, phase: ProgressPhase) -> ProgressPayload {
        let total_known = self.total_known.load(Ordering::SeqCst);
        let bytes_written = self.bytes_written.load(Ordering::SeqCst);
        let files_processed = self.files_processed.load(Ordering::SeqCst) as u32;

        ProgressPayload {
            percentage: self.percentage(),
            bytes_written,
            total_bytes: if total_known {
                Some(self.total_bytes.load(Ordering::SeqCst))
            } else {
                None
            },
            eta_seconds: self.eta_seconds(),
            current_file,
            files_processed,
            total_files: if total_known {
                Some(self.total_files.load(Ordering::SeqCst) as u32)
            } else {
                None
            },
            phase,
        }
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress emitter that sends events to the Tauri frontend
pub struct ProgressEmitter {
    window: Window,
    tracker: Arc<ProgressTracker>,
    event_name: String,
}

impl ProgressEmitter {
    /// Create a new progress emitter
    pub fn new(window: Window, tracker: Arc<ProgressTracker>, event_name: impl Into<String>) -> Self {
        Self {
            window,
            tracker,
            event_name: event_name.into(),
        }
    }

    /// Emit progress if throttle allows, returns true if emitted
    pub fn emit_progress(&self, current_file: Option<String>, phase: ProgressPhase) -> bool {
        if !self.tracker.should_emit() {
            return false;
        }
        self.emit_progress_forced(current_file, phase)
    }

    /// Emit progress regardless of throttle
    pub fn emit_progress_forced(&self, current_file: Option<String>, phase: ProgressPhase) -> bool {
        let payload = self.tracker.build_payload(current_file, phase);
        match self.window.emit(&self.event_name, &payload) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[ProgressEmitter] Failed to emit event: {}", e);
                false
            }
        }
    }

    /// Emit a completion event
    pub fn emit_complete(&self) {
        self.tracker.force_next_emit();
        let payload = self.tracker.build_payload(None, ProgressPhase::Complete);
        if let Err(e) = self.window.emit(&self.event_name, &payload) {
            eprintln!("[ProgressEmitter] Failed to emit completion event: {}", e);
        }
    }

    /// Check if operation was cancelled
    pub fn is_cancelled(&self) -> bool {
        self.tracker.is_cancelled()
    }
}

/// Calculate total size of a path (file or directory)
pub fn calculate_total_size(path: &std::path::Path) -> std::io::Result<(u64, u32)> {
    let mut total_bytes: u64 = 0;
    let mut total_files: u32 = 0;

    if path.is_file() {
        let metadata = std::fs::metadata(path)?;
        return Ok((metadata.len(), 1));
    }

    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Ok(metadata) = entry.metadata() {
                    total_bytes += metadata.len();
                    total_files += 1;
                }
            }
        }
    }

    Ok((total_bytes, total_files))
}

/// Cancellation error for when operation is cancelled by user
#[derive(Debug)]
pub struct CancellationError;

impl std::fmt::Display for CancellationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Operation cancelled by user")
    }
}

impl std::error::Error for CancellationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_tracker_percentage() {
        let tracker = ProgressTracker::with_total(1000, 10);
        assert_eq!(tracker.percentage(), Some(0.0));

        tracker.add_bytes(250);
        assert_eq!(tracker.percentage(), Some(25.0));

        tracker.add_bytes(250);
        assert_eq!(tracker.percentage(), Some(50.0));

        tracker.set_bytes_written(1000);
        assert_eq!(tracker.percentage(), Some(100.0));
    }

    #[test]
    fn test_progress_tracker_unknown_total() {
        let tracker = ProgressTracker::new();
        assert_eq!(tracker.percentage(), None);

        tracker.add_bytes(500);
        assert_eq!(tracker.percentage(), None);

        // Can set total later
        tracker.set_total(1000, 5);
        assert_eq!(tracker.percentage(), Some(50.0));
    }

    #[test]
    fn test_progress_tracker_cancellation() {
        let tracker = ProgressTracker::new();
        assert!(!tracker.is_cancelled());

        tracker.cancel();
        assert!(tracker.is_cancelled());
    }

    #[test]
    fn test_progress_tracker_files() {
        let tracker = ProgressTracker::with_total(1000, 5);

        let payload = tracker.build_payload(Some("file1.txt".to_string()), ProgressPhase::Compressing);
        assert_eq!(payload.files_processed, 0);
        assert_eq!(payload.total_files, Some(5));

        tracker.increment_files();
        tracker.increment_files();

        let payload = tracker.build_payload(Some("file3.txt".to_string()), ProgressPhase::Compressing);
        assert_eq!(payload.files_processed, 2);
    }

    #[test]
    fn test_throttling() {
        let tracker = ProgressTracker::new();

        // First call should always emit (time is initialized to now)
        // Force to start fresh
        tracker.force_next_emit();
        assert!(tracker.should_emit());

        // Immediate second call should be throttled
        assert!(!tracker.should_emit());

        // Wait a tiny bit and force next emit
        std::thread::sleep(std::time::Duration::from_millis(1));
        tracker.force_next_emit();
        assert!(tracker.should_emit());
    }
}
