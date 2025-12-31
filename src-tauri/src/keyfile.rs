use crate::error::{Result, TimeLockerError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Key file structure with YAML frontmatter and encrypted body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFile {
    /// Metadata from YAML frontmatter
    pub metadata: KeyMetadata,
    /// Encrypted AGE content
    pub encrypted_body: String,
    /// Path to the key file
    pub file_path: Option<PathBuf>,
}

/// Metadata stored in YAML frontmatter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMetadata {
    pub locked: bool,
    pub created: DateTime<Utc>,
    pub unlocks: DateTime<Utc>,
    pub duration: String,
    pub original_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_path: Option<String>,
}

impl KeyFile {
    /// Create a new key file
    ///
    /// # Arguments
    /// * `original_file` - Name of the original file
    /// * `duration` - Duration string (e.g., "2026-07-01")
    /// * `unlocks` - Unlock date/time
    /// * `encrypted_content` - AGE encrypted content
    pub fn create(
        original_file: String,
        duration: String,
        unlocks: DateTime<Utc>,
        encrypted_content: String,
    ) -> Self {
        Self {
            metadata: KeyMetadata {
                locked: true,
                created: Utc::now(),
                unlocks,
                duration,
                original_file,
                archive_path: None,
            },
            encrypted_body: encrypted_content,
            file_path: None,
        }
    }

    /// Parse a key file from content
    ///
    /// Expected format:
    /// ```yaml
    /// ---
    /// locked: true
    /// created: 2025-12-20 12:17:42 UTC
    /// unlocks: 2026-07-01 06:00:00 UTC
    /// duration: 2026-07-01
    /// original_file: vault-1.md
    /// ---
    ///
    /// -----BEGIN AGE ENCRYPTED FILE-----
    /// <encrypted content>
    /// -----END AGE ENCRYPTED FILE-----
    /// ```
    pub fn parse(content: &str) -> Result<Self> {
        // Split frontmatter and body
        let parts: Vec<&str> = content.splitn(3, "---").collect();

        if parts.len() < 3 {
            eprintln!("[KeyFile::parse] Not enough parts after splitting by '---': {}", parts.len());
            return Err(TimeLockerError::InvalidKeyFile);
        }

        // Parse YAML frontmatter (parts[1])
        let yaml_str = parts[1].trim();
        let metadata: KeyMetadata = serde_yaml::from_str(yaml_str)
            .map_err(|e| {
                eprintln!("[KeyFile::parse] YAML parse error: {}", e);
                TimeLockerError::YamlParse(e.to_string())
            })?;

        // Extract encrypted body (parts[2])
        let body_str = parts[2].trim();

        // Check for AGE markers and extract the base64 content
        let encrypted_body = if body_str.contains("-----BEGIN AGE ENCRYPTED FILE-----") {
            // Extract content between markers
            let start_marker = "-----BEGIN AGE ENCRYPTED FILE-----";
            let end_marker = "-----END AGE ENCRYPTED FILE-----";

            if let Some(start_idx) = body_str.find(start_marker) {
                let after_start = &body_str[start_idx + start_marker.len()..];
                if let Some(end_idx) = after_start.find(end_marker) {
                    after_start[..end_idx].trim().to_string()
                } else {
                    // No end marker, just take everything after start marker
                    after_start.trim().to_string()
                }
            } else {
                body_str.to_string()
            }
        } else {
            // No markers, assume raw base64
            body_str.to_string()
        };

        eprintln!("[KeyFile::parse] Successfully parsed key file for: {}", metadata.original_file);

        Ok(Self {
            metadata,
            encrypted_body,
            file_path: None,
        })
    }

    /// Save key file to disk
    ///
    /// # Arguments
    /// * `path` - Destination path (should end with .key.md)
    pub fn save(&mut self, path: &Path) -> Result<()> {
        let content = self.to_string();
        fs::write(path, content)?;
        self.file_path = Some(path.to_path_buf());
        Ok(())
    }

    /// Convert to string format with YAML frontmatter
    pub fn to_string(&self) -> String {
        let yaml = serde_yaml::to_string(&self.metadata)
            .unwrap_or_else(|_| String::from("# Error serializing metadata\n"));

        // Wrap encrypted body in AGE-style markers for consistent format
        let body = if self.encrypted_body.contains("-----BEGIN AGE ENCRYPTED FILE-----") {
            self.encrypted_body.clone()
        } else {
            format!(
                "-----BEGIN AGE ENCRYPTED FILE-----\n{}\n-----END AGE ENCRYPTED FILE-----",
                self.encrypted_body
            )
        };

        format!(
            "---\n{yaml}---\n\n{body}",
            yaml = yaml,
            body = body
        )
    }

    /// Check if the time lock has expired
    pub fn is_unlockable(&self) -> bool {
        Utc::now() >= self.metadata.unlocks
    }

    /// Get time remaining until unlock
    pub fn time_until_unlock(&self) -> chrono::Duration {
        self.metadata.unlocks - Utc::now()
    }
}

/// Scan a directory for all key files (.key.md or -key.md)
///
/// # Arguments
/// * `dir` - Directory to scan (recursively)
///
/// # Returns
/// Result containing vector of parsed KeyFile objects
pub fn scan_directory(dir: &Path) -> Result<Vec<KeyFile>> {
    let mut keyfiles = Vec::new();

    if !dir.exists() || !dir.is_dir() {
        eprintln!("[scan_directory] Directory does not exist or is not a dir: {:?}", dir);
        return Ok(keyfiles);
    }

    eprintln!("[scan_directory] Scanning directory: {:?}", dir);

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
    {
        let path = entry.path();

        // Check if filename contains "key.md" (matches both ".key.md" and "-key.md")
        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
            if file_name.ends_with("key.md") || file_name.ends_with(".key.md") {
                eprintln!("[scan_directory] Found potential key file: {:?}", path);
                match fs::read_to_string(path) {
                    Ok(content) => {
                        match KeyFile::parse(&content) {
                            Ok(mut keyfile) => {
                                eprintln!("[scan_directory] Successfully parsed: {:?}", path);
                                keyfile.file_path = Some(path.to_path_buf());
                                keyfiles.push(keyfile);
                            }
                            Err(e) => {
                                eprintln!("[scan_directory] Failed to parse {:?}: {:?}", path, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[scan_directory] Failed to read {:?}: {:?}", path, e);
                    }
                }
            }
        }
    }

    eprintln!("[scan_directory] Found {} key files", keyfiles.len());
    Ok(keyfiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_keyfile_parse_with_age_markers() {
        let content = r#"---
locked: true
created: 2025-12-20T12:17:42Z
unlocks: 2026-07-01T06:00:00Z
duration: "2026-07-01"
original_file: vault-1.md
---

-----BEGIN AGE ENCRYPTED FILE-----
YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IHRsb2NrIDY2ZDcwMDAwIDEgMTU5
-----END AGE ENCRYPTED FILE-----
"#;

        let keyfile = KeyFile::parse(content).unwrap();
        assert_eq!(keyfile.metadata.original_file, "vault-1.md");
        assert_eq!(keyfile.metadata.duration, "2026-07-01");
        assert!(keyfile.metadata.locked);
        // Should extract just the base64 content
        assert_eq!(keyfile.encrypted_body, "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IHRsb2NrIDY2ZDcwMDAwIDEgMTU5");
    }

    #[test]
    fn test_keyfile_parse_without_age_markers() {
        let content = r#"---
locked: true
created: 2025-12-20T12:17:42Z
unlocks: 2026-07-01T06:00:00Z
duration: "2026-07-01"
original_file: test-file.txt
---

SGVsbG8gV29ybGQgYmFzZTY0IGVuY29kZWQ=
"#;

        let keyfile = KeyFile::parse(content).unwrap();
        assert_eq!(keyfile.metadata.original_file, "test-file.txt");
        assert_eq!(keyfile.encrypted_body, "SGVsbG8gV29ybGQgYmFzZTY0IGVuY29kZWQ=");
    }

    #[test]
    fn test_keyfile_create_and_save() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("test_keyfile");
        fs::create_dir_all(&temp_dir)?;

        let unlocks = Utc::now() + Duration::days(30);
        // Create with raw base64 (no AGE markers)
        let mut keyfile = KeyFile::create(
            "test.txt".to_string(),
            "30d".to_string(),
            unlocks,
            "SGVsbG8gV29ybGQgYmFzZTY0".to_string(),
        );

        let key_path = temp_dir.join("test.key.md");
        keyfile.save(&key_path)?;

        assert!(key_path.exists());

        // Read and verify it has AGE markers added
        let content = fs::read_to_string(&key_path)?;
        assert!(content.contains("-----BEGIN AGE ENCRYPTED FILE-----"));
        assert!(content.contains("-----END AGE ENCRYPTED FILE-----"));

        // Parse should extract the base64 content
        let parsed = KeyFile::parse(&content)?;
        assert_eq!(parsed.metadata.original_file, "test.txt");
        assert_eq!(parsed.encrypted_body, "SGVsbG8gV29ybGQgYmFzZTY0");

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }
}
