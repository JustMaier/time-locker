// cli.rs - Command Line Interface for Time Locker

use crate::crypto;
use crate::error::{Result, TimeLockerError};
use crate::tlock_format::{self, TlockArchive, TlockMetadata};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Time Locker - Secure time-locked file encryption
#[derive(Parser, Debug)]
#[command(name = "timelocker")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Lock a file or directory with time-based encryption
    Lock {
        /// Path to file or directory to lock
        source: PathBuf,

        /// Date/time when the file can be unlocked (RFC3339 or "YYYY-MM-DD" or "YYYY-MM-DD HH:MM")
        #[arg(long, short = 'u')]
        unlock_at: String,

        /// Vault directory to store the locked file
        #[arg(long, short = 'v')]
        vault: Option<PathBuf>,

        /// Delete the original file after locking
        #[arg(long, short = 'd')]
        delete_original: bool,
    },

    /// Unlock a time-locked file
    Unlock {
        /// Path to the .7z.tlock file to unlock
        file: PathBuf,

        /// Output directory for extracted files
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Display metadata from a .7z.tlock file
    Info {
        /// Path to the .7z.tlock file
        file: PathBuf,
    },

    /// List all .7z.tlock files in vault(s)
    List {
        /// Vault directory to scan (defaults to current directory)
        #[arg(long, short = 'v')]
        vault: Option<PathBuf>,
    },

    /// Migrate old .key.md format to new .7z.tlock format
    Migrate {
        /// Path to the .key.md file
        keyfile: PathBuf,

        /// Delete old files after successful migration
        #[arg(long, short = 'd')]
        delete_old: bool,
    },
}

/// Run the CLI application
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Some(cmd) => match execute_command(cmd) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: {}", e);
                ExitCode::FAILURE
            }
        },
        None => {
            // No command specified - launch GUI
            ExitCode::from(2) // Special code to indicate GUI mode
        }
    }
}

/// Execute a CLI command
fn execute_command(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Lock {
            source,
            unlock_at,
            vault,
            delete_original,
        } => cmd_lock(&source, &unlock_at, vault.as_deref(), delete_original),

        Commands::Unlock { file, output } => cmd_unlock(&file, output.as_deref()),

        Commands::Info { file } => cmd_info(&file),

        Commands::List { vault } => cmd_list(vault.as_deref()),

        Commands::Migrate { keyfile, delete_old } => cmd_migrate(&keyfile, delete_old),
    }
}

/// Lock command implementation
fn cmd_lock(
    source: &Path,
    unlock_at: &str,
    vault: Option<&Path>,
    delete_original: bool,
) -> Result<()> {
    // Validate source exists
    if !source.exists() {
        return Err(TimeLockerError::FileNotFound(source.display().to_string()));
    }

    // Parse unlock time
    let unlock_datetime = parse_datetime(unlock_at)?;

    if unlock_datetime <= Utc::now() {
        return Err(TimeLockerError::Parse(
            "Unlock time must be in the future".to_string(),
        ));
    }

    println!("Locking: {}", source.display());
    println!(
        "Unlock at: {}",
        unlock_datetime
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
    );

    // Generate password
    print!("Generating secure password... ");
    io::stdout().flush()?;
    let password = crypto::generate_password(32);
    println!("done");

    // Encrypt the password with time-lock
    print!("Encrypting password with time-lock... ");
    io::stdout().flush()?;
    let encrypted_password = crypto::encrypt_with_tlock(&password, unlock_datetime)?;
    println!("done");

    // Create metadata
    let original_filename = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let duration_str = unlock_datetime.format("%Y-%m-%d").to_string();
    let mut metadata = TlockMetadata::new(
        original_filename.clone(),
        duration_str,
        unlock_datetime,
        None, // drand_round - not used in current implementation
        Some(encrypted_password),
    );
    metadata.is_directory = source.is_dir();

    // Create .7z.tlock file
    print!("Creating encrypted archive... ");
    io::stdout().flush()?;
    let tlock_path = TlockArchive::create(source, metadata, &password)?;
    println!("done");

    // Move to vault if specified
    let final_path = if let Some(vault_dir) = vault {
        if vault_dir.exists() && vault_dir.is_dir() {
            let filename = tlock_path.file_name().unwrap();
            let dest_path = vault_dir.join(filename);
            print!("Moving to vault... ");
            io::stdout().flush()?;
            fs::rename(&tlock_path, &dest_path)?;
            println!("done");
            dest_path
        } else {
            println!("Warning: Vault directory does not exist, keeping in place");
            tlock_path
        }
    } else {
        tlock_path
    };

    // Delete original if requested
    if delete_original {
        print!("Verifying archive... ");
        io::stdout().flush()?;
        if TlockArchive::validate(&final_path)? {
            println!("done");
            print!("Deleting original... ");
            io::stdout().flush()?;
            if source.is_dir() {
                fs::remove_dir_all(source)?;
            } else {
                fs::remove_file(source)?;
            }
            println!("done");
        } else {
            println!("failed");
            println!("Warning: Archive verification failed, original not deleted");
        }
    }

    println!();
    println!("Success! Created: {}", final_path.display());
    println!(
        "File will be unlockable after: {}",
        unlock_datetime
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );

    Ok(())
}

/// Unlock command implementation
fn cmd_unlock(file: &Path, output: Option<&Path>) -> Result<()> {
    // Validate file exists
    if !file.exists() {
        return Err(TimeLockerError::FileNotFound(file.display().to_string()));
    }

    // Read metadata
    print!("Reading metadata... ");
    io::stdout().flush()?;
    let archive = TlockArchive::read_metadata(file)?;
    let metadata = archive
        .get_metadata()
        .ok_or_else(|| TimeLockerError::Parse("Failed to read metadata".to_string()))?;
    println!("done");

    println!("Original file: {}", metadata.original_file);
    println!(
        "Locked at: {}",
        metadata
            .created
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );
    println!(
        "Unlock time: {}",
        metadata
            .unlocks
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );

    // Check if unlockable
    if !metadata.is_unlockable() {
        let remaining = metadata.time_until_unlock();
        let hours = remaining.num_hours();
        let minutes = remaining.num_minutes() % 60;
        let seconds = remaining.num_seconds() % 60;

        println!();
        println!("Time lock still active!");
        println!("Remaining: {}h {}m {}s", hours, minutes, seconds);
        return Err(TimeLockerError::TimeLockActive);
    }

    println!("Time lock expired - proceeding with unlock");

    // Get encrypted password from metadata
    let encrypted_password = metadata
        .encrypted_key
        .as_ref()
        .ok_or_else(|| TimeLockerError::MissingField("encrypted_key".to_string()))?;

    // Decrypt password
    print!("Decrypting password... ");
    io::stdout().flush()?;
    let password = crypto::decrypt_with_tlock(encrypted_password, metadata.unlocks)?;
    println!("done");

    // Determine output directory
    let output_dir = match output {
        Some(p) => p.to_path_buf(),
        None => {
            let parent = file.parent().unwrap_or_else(|| Path::new("."));
            parent.join(format!("unlocked_{}", metadata.original_file))
        }
    };

    // Extract the archive
    print!("Extracting files... ");
    io::stdout().flush()?;
    TlockArchive::extract(file, &password, &output_dir)?;
    println!("done");

    println!();
    println!("Success! Extracted to: {}", output_dir.display());

    Ok(())
}

/// Info command implementation
fn cmd_info(file: &Path) -> Result<()> {
    if !file.exists() {
        return Err(TimeLockerError::FileNotFound(file.display().to_string()));
    }

    let archive = TlockArchive::read_metadata(file)?;
    let metadata = archive
        .get_metadata()
        .ok_or_else(|| TimeLockerError::Parse("Failed to read metadata".to_string()))?;

    println!("Time-Locked File Information");
    println!("============================");
    println!("File: {}", file.display());
    println!("Original name: {}", metadata.original_file);
    println!("Type: {}", if metadata.is_directory { "Directory" } else { "File" });
    println!();
    println!(
        "Created: {}",
        metadata
            .created
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
    );
    println!(
        "Unlocks: {}",
        metadata
            .unlocks
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
    );
    println!("Duration: {}", metadata.duration);
    println!();

    if metadata.is_unlockable() {
        println!("Status: UNLOCKABLE");
        println!("The time lock has expired. This file can now be unlocked.");
    } else {
        let remaining = metadata.time_until_unlock();
        let days = remaining.num_days();
        let hours = remaining.num_hours() % 24;
        let minutes = remaining.num_minutes() % 60;

        println!("Status: LOCKED");
        println!("Time remaining: {}d {}h {}m", days, hours, minutes);
    }

    if let Some(ref drand_round) = metadata.drand_round {
        println!();
        println!("Drand round: {}", drand_round);
    }

    Ok(())
}

/// List command implementation
fn cmd_list(vault: Option<&Path>) -> Result<()> {
    let scan_dir = vault
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    println!("Scanning: {}", scan_dir.display());
    println!();

    let archives = tlock_format::scan_tlock_files(&scan_dir)?;

    if archives.is_empty() {
        println!("No .7z.tlock files found.");
        return Ok(());
    }

    println!(
        "{:<40} {:<12} {:<20} {}",
        "File", "Status", "Unlocks At", "Original Name"
    );
    println!("{}", "-".repeat(90));

    for archive in archives {
        if let Some(metadata) = archive.get_metadata() {
            let filename = archive
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("?");

            let status = if metadata.is_unlockable() {
                "UNLOCKABLE"
            } else {
                "LOCKED"
            };

            let unlock_time = metadata.unlocks.with_timezone(&Local).format("%Y-%m-%d %H:%M");

            // Truncate filename if too long
            let display_name = if filename.len() > 38 {
                format!("{}...", &filename[..35])
            } else {
                filename.to_string()
            };

            println!(
                "{:<40} {:<12} {:<20} {}",
                display_name, status, unlock_time, metadata.original_file
            );
        }
    }

    Ok(())
}

/// Migrate command implementation
fn cmd_migrate(keyfile: &Path, delete_old: bool) -> Result<()> {
    if !keyfile.exists() {
        return Err(TimeLockerError::FileNotFound(keyfile.display().to_string()));
    }

    println!("Migrating: {}", keyfile.display());

    // Read old format key file
    let content = fs::read_to_string(keyfile)?;
    let old_keyfile = crate::keyfile::KeyFile::parse(&content)?;

    let archive_path_str = old_keyfile
        .metadata
        .archive_path
        .as_ref()
        .ok_or_else(|| TimeLockerError::MissingField("archive_path".to_string()))?;
    let archive_path = Path::new(archive_path_str);

    if !archive_path.exists() {
        return Err(TimeLockerError::FileNotFound(archive_path_str.clone()));
    }

    println!("Archive: {}", archive_path.display());
    println!("Original file: {}", old_keyfile.metadata.original_file);
    println!(
        "Unlock time: {}",
        old_keyfile
            .metadata
            .unlocks
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    );

    // Create new tlock metadata from old format
    let metadata = TlockMetadata::new(
        old_keyfile.metadata.original_file.clone(),
        old_keyfile.metadata.duration.clone(),
        old_keyfile.metadata.unlocks,
        None,
        Some(old_keyfile.encrypted_body.clone()),
    );

    // Read the old 7z archive
    print!("Reading archive... ");
    io::stdout().flush()?;
    let archive_data = fs::read(archive_path)?;
    println!("done ({} bytes)", archive_data.len());

    // Create the new .7z.tlock file
    // The tlock file combines: header + metadata + 7z payload
    let tlock_path = archive_path.with_extension("7z.tlock");
    print!("Creating .7z.tlock file... ");
    io::stdout().flush()?;

    // Serialize metadata
    let metadata_json = serde_json::to_vec(&metadata)
        .map_err(|e| TimeLockerError::Parse(format!("Failed to serialize metadata: {}", e)))?;

    // Write the tlock file: header + metadata + payload
    let mut output = fs::File::create(&tlock_path)?;

    // Write header
    output.write_all(tlock_format::TLOCK_MAGIC)?;
    output.write_all(&[tlock_format::TLOCK_VERSION])?;
    output.write_all(&(metadata_json.len() as u32).to_le_bytes())?;
    output.write_all(&[0u8; 12])?; // Reserved bytes

    // Write metadata
    output.write_all(&metadata_json)?;

    // Write 7z payload
    output.write_all(&archive_data)?;
    output.flush()?;

    println!("done");
    println!("Created: {}", tlock_path.display());

    // Delete old files if requested
    if delete_old {
        print!("Verifying new file... ");
        io::stdout().flush()?;
        if TlockArchive::validate(&tlock_path)? {
            println!("done");
            print!("Deleting old files... ");
            io::stdout().flush()?;

            // Delete key file
            fs::remove_file(keyfile)?;

            // Delete old archive
            if archive_path.exists() {
                fs::remove_file(archive_path)?;
            }

            println!("done");
        } else {
            println!("failed");
            println!("Warning: Verification failed, old files not deleted");
        }
    }

    println!();
    println!("Migration complete!");

    Ok(())
}

/// Parse datetime from various formats
fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    // Try RFC3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try "YYYY-MM-DD HH:MM:SS"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        let local = Local
            .from_local_datetime(&dt)
            .single()
            .ok_or_else(|| TimeLockerError::Parse("Ambiguous datetime".to_string()))?;
        return Ok(local.with_timezone(&Utc));
    }

    // Try "YYYY-MM-DD HH:MM"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        let local = Local
            .from_local_datetime(&dt)
            .single()
            .ok_or_else(|| TimeLockerError::Parse("Ambiguous datetime".to_string()))?;
        return Ok(local.with_timezone(&Utc));
    }

    // Try "YYYY-MM-DD" (default to midnight)
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| TimeLockerError::Parse("Invalid time".to_string()))?;
        let local = Local
            .from_local_datetime(&dt)
            .single()
            .ok_or_else(|| TimeLockerError::Parse("Ambiguous datetime".to_string()))?;
        return Ok(local.with_timezone(&Utc));
    }

    Err(TimeLockerError::Parse(format!(
        "Cannot parse datetime: '{}'. Use RFC3339 (2025-12-31T23:59:59Z) or YYYY-MM-DD or YYYY-MM-DD HH:MM",
        s
    )))
}

/// Check if CLI arguments were provided (excluding the program name)
pub fn has_cli_args() -> bool {
    std::env::args().count() > 1
}
