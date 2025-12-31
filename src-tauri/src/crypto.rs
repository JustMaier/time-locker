use crate::error::{Result, TimeLockerError};
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use chrono::{DateTime, Utc};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::io::Cursor;

// ============================================================================
// DRAND QUICKNET BEACON CONFIGURATION
// ============================================================================
// Quicknet is the recommended unchained beacon for tlock encryption.
// It produces randomness every 3 seconds with BLS signatures on G1.
// See: https://drand.love/developer/http-api/

/// Drand Quicknet chain hash (hex encoded)
const QUICKNET_CHAIN_HASH: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";

/// Drand Quicknet public key (hex encoded BLS12-381 G2 point)
const QUICKNET_PUBLIC_KEY: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a";

/// Genesis time of Quicknet (Unix timestamp of round 1)
const QUICKNET_GENESIS_TIME: u64 = 1692803367;

/// Period between rounds in seconds
const QUICKNET_PERIOD: u64 = 3;

/// Drand API endpoints (multiple for redundancy)
const DRAND_ENDPOINTS: &[&str] = &[
    "https://api.drand.sh",
    "https://drand.cloudflare.com",
];

// ============================================================================
// ROUND CALCULATION
// ============================================================================

/// Calculate the drand round number for a given Unix timestamp.
///
/// The formula is: round = ((timestamp - genesis_time) / period) + 1
/// Round 1 occurs at genesis_time.
///
/// # Arguments
/// * `unix_timestamp` - Unix timestamp in seconds
///
/// # Returns
/// The round number that will be available at or after the given timestamp
pub fn timestamp_to_round(unix_timestamp: u64) -> u64 {
    if unix_timestamp <= QUICKNET_GENESIS_TIME {
        return 1;
    }
    let elapsed = unix_timestamp - QUICKNET_GENESIS_TIME;
    (elapsed / QUICKNET_PERIOD) + 1
}

/// Calculate the Unix timestamp when a specific round becomes available.
///
/// # Arguments
/// * `round` - The drand round number
///
/// # Returns
/// Unix timestamp when the round signature will be published
pub fn round_to_timestamp(round: u64) -> u64 {
    if round <= 1 {
        return QUICKNET_GENESIS_TIME;
    }
    QUICKNET_GENESIS_TIME + ((round - 1) * QUICKNET_PERIOD)
}

/// Convert a DateTime to the corresponding drand round number.
/// Rounds up to ensure the unlock time has definitely passed.
///
/// # Arguments
/// * `datetime` - The unlock DateTime in UTC
///
/// # Returns
/// The round number to encrypt for
pub fn datetime_to_round(datetime: DateTime<Utc>) -> u64 {
    let timestamp = datetime.timestamp() as u64;
    // Add 1 to ensure we're past the unlock time when this round is available
    timestamp_to_round(timestamp) + 1
}

// ============================================================================
// ENCRYPTION
// ============================================================================

/// Generate a secure random password
pub fn generate_password(length: u32) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length as usize)
        .map(char::from)
        .collect()
}

/// Encrypt data using tlock timelock encryption.
///
/// This uses the drand Quicknet beacon for cryptographic time-locking.
/// The encrypted data can ONLY be decrypted after the specified drand round
/// has been published, which corresponds to the unlock_time.
///
/// # Arguments
/// * `password` - The password/data to encrypt
/// * `unlock_time` - The DateTime when decryption should become possible
///
/// # Returns
/// Base64-encoded tlock ciphertext with round metadata prepended
///
/// # Security
/// This is cryptographically enforced - no one (not even the encryptor) can
/// decrypt the data until the drand network publishes the signature for the
/// target round. The security is based on BLS threshold signatures.
pub fn encrypt_with_tlock(password: &str, unlock_time: DateTime<Utc>) -> Result<String> {
    // Calculate the target drand round for this unlock time
    let round = datetime_to_round(unlock_time);

    // Decode chain hash and public key from hex
    let chain_hash = hex::decode(QUICKNET_CHAIN_HASH)
        .map_err(|e| TimeLockerError::Encryption(format!("Invalid chain hash: {}", e)))?;

    let public_key = hex::decode(QUICKNET_PUBLIC_KEY)
        .map_err(|e| TimeLockerError::Encryption(format!("Invalid public key: {}", e)))?;

    // Prepare input and output buffers
    let input = Cursor::new(password.as_bytes());
    let mut output = Vec::new();

    // Perform tlock encryption
    // This encrypts the data such that it can only be decrypted with the
    // BLS signature for the specified round
    tlock_age::encrypt(&mut output, input, &chain_hash, &public_key, round)
        .map_err(|e| TimeLockerError::Encryption(format!("Tlock encryption failed: {}", e)))?;

    // Prepend round number (8 bytes big-endian) for decryption reference
    let mut result = round.to_be_bytes().to_vec();
    result.extend_from_slice(&output);

    // Encode as base64 for safe storage
    Ok(BASE64.encode(&result))
}

// ============================================================================
// DECRYPTION
// ============================================================================

/// Fetch the drand beacon signature for a specific round.
///
/// Tries multiple endpoints for redundancy.
///
/// # Arguments
/// * `round` - The round number to fetch
///
/// # Returns
/// The BLS signature bytes for the round
fn fetch_drand_signature(round: u64) -> Result<Vec<u8>> {
    use drand_core::HttpClient;

    let chain_path = format!("/{}", QUICKNET_CHAIN_HASH);

    for endpoint in DRAND_ENDPOINTS {
        let url = format!("{}{}", endpoint, chain_path);

        match HttpClient::new(&url, None) {
            Ok(client) => {
                match client.get(round) {
                    Ok(beacon) => {
                        // Extract signature from the beacon
                        // The beacon contains the BLS signature we need for decryption
                        return Ok(beacon.signature().to_vec());
                    }
                    Err(e) => {
                        // Try next endpoint
                        eprintln!("Drand endpoint {} failed for round {}: {}", endpoint, round, e);
                        continue;
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to create client for {}: {}", endpoint, e);
                continue;
            }
        }
    }

    Err(TimeLockerError::Decryption(format!(
        "Failed to fetch drand signature for round {} from all endpoints. \
         The round may not have been published yet (time lock still active).",
        round
    )))
}

/// Check if a specific drand round is available (time has passed).
///
/// # Arguments
/// * `round` - The round number to check
///
/// # Returns
/// true if the round signature is available, false otherwise
pub fn is_round_available(round: u64) -> bool {
    let round_time = round_to_timestamp(round);
    let now = Utc::now().timestamp() as u64;
    now >= round_time
}

/// Decrypt time-locked data using tlock.
///
/// This fetches the drand signature for the encrypted round and uses it
/// to decrypt the data. Will fail if the round hasn't been published yet.
///
/// # Arguments
/// * `encrypted` - The base64-encoded tlock ciphertext (with round prepended)
/// * `unlock_time` - The expected unlock time (used for verification)
///
/// # Returns
/// The decrypted password/data
///
/// # Errors
/// - `TimeLockActive` if the drand round hasn't been published yet
/// - `Decryption` if the data is corrupted or signature fetch fails
pub fn decrypt_with_tlock(encrypted: &str, unlock_time: DateTime<Utc>) -> Result<String> {
    // Decode from base64
    let encrypted_bytes = BASE64.decode(encrypted)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid base64: {}", e)))?;

    // Extract round number (first 8 bytes)
    if encrypted_bytes.len() < 9 {
        return Err(TimeLockerError::Decryption("Invalid encrypted data: too short".to_string()));
    }

    let round_bytes: [u8; 8] = encrypted_bytes[0..8].try_into()
        .map_err(|_| TimeLockerError::Decryption("Invalid round bytes".to_string()))?;
    let round = u64::from_be_bytes(round_bytes);

    let ciphertext = &encrypted_bytes[8..];

    // Check if the unlock time has passed (optional early check)
    let expected_round = datetime_to_round(unlock_time);
    if round != expected_round {
        eprintln!("Warning: Round mismatch. Stored: {}, Expected: {}", round, expected_round);
    }

    // Check if we can even attempt decryption
    if !is_round_available(round) {
        return Err(TimeLockerError::TimeLockActive);
    }

    // Fetch the drand signature for this round
    let signature = fetch_drand_signature(round)?;

    // Decode chain hash
    let chain_hash = hex::decode(QUICKNET_CHAIN_HASH)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid chain hash: {}", e)))?;

    // Prepare input and output buffers
    let input = Cursor::new(ciphertext);
    let mut output = Vec::new();

    // Perform tlock decryption using the drand signature
    tlock_age::decrypt(&mut output, input, &chain_hash, &signature)
        .map_err(|e| TimeLockerError::Decryption(format!("Tlock decryption failed: {}", e)))?;

    // Convert to string
    String::from_utf8(output)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid UTF-8 in decrypted data: {}", e)))
}

/// Decrypt time-locked data by extracting round from the ciphertext.
///
/// This version doesn't require the unlock_time parameter - it extracts
/// the round number from the encrypted data itself.
///
/// # Arguments
/// * `encrypted` - The base64-encoded tlock ciphertext (with round prepended)
///
/// # Returns
/// The decrypted password/data
pub fn decrypt_with_tlock_auto(encrypted: &str) -> Result<String> {
    // Decode from base64
    let encrypted_bytes = BASE64.decode(encrypted)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid base64: {}", e)))?;

    // Extract round number (first 8 bytes)
    if encrypted_bytes.len() < 9 {
        return Err(TimeLockerError::Decryption("Invalid encrypted data: too short".to_string()));
    }

    let round_bytes: [u8; 8] = encrypted_bytes[0..8].try_into()
        .map_err(|_| TimeLockerError::Decryption("Invalid round bytes".to_string()))?;
    let round = u64::from_be_bytes(round_bytes);

    let ciphertext = &encrypted_bytes[8..];

    // Check if we can even attempt decryption
    if !is_round_available(round) {
        return Err(TimeLockerError::TimeLockActive);
    }

    // Fetch the drand signature for this round
    let signature = fetch_drand_signature(round)?;

    // Decode chain hash
    let chain_hash = hex::decode(QUICKNET_CHAIN_HASH)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid chain hash: {}", e)))?;

    // Prepare input and output buffers
    let input = Cursor::new(ciphertext);
    let mut output = Vec::new();

    // Perform tlock decryption using the drand signature
    tlock_age::decrypt(&mut output, input, &chain_hash, &signature)
        .map_err(|e| TimeLockerError::Decryption(format!("Tlock decryption failed: {}", e)))?;

    // Convert to string
    String::from_utf8(output)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid UTF-8 in decrypted data: {}", e)))
}

/// Get information about an encrypted tlock ciphertext.
///
/// # Arguments
/// * `encrypted` - The base64-encoded tlock ciphertext
///
/// # Returns
/// Tuple of (round_number, unlock_timestamp, is_available)
pub fn get_tlock_info(encrypted: &str) -> Result<(u64, u64, bool)> {
    let encrypted_bytes = BASE64.decode(encrypted)
        .map_err(|e| TimeLockerError::Decryption(format!("Invalid base64: {}", e)))?;

    if encrypted_bytes.len() < 8 {
        return Err(TimeLockerError::Decryption("Invalid encrypted data".to_string()));
    }

    let round_bytes: [u8; 8] = encrypted_bytes[0..8].try_into()
        .map_err(|_| TimeLockerError::Decryption("Invalid round bytes".to_string()))?;
    let round = u64::from_be_bytes(round_bytes);
    let unlock_time = round_to_timestamp(round);
    let available = is_round_available(round);

    Ok((round, unlock_time, available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_password() {
        let password = generate_password(16);
        assert_eq!(password.len(), 16);
        assert!(password.chars().all(|c| c.is_alphanumeric()));
    }

    #[test]
    fn test_generate_password_different_lengths() {
        for length in [8, 16, 32, 64] {
            let password = generate_password(length);
            assert_eq!(password.len(), length as usize);
        }
    }

    #[test]
    fn test_timestamp_to_round() {
        // Genesis time should give round 1
        assert_eq!(timestamp_to_round(QUICKNET_GENESIS_TIME), 1);

        // 3 seconds after genesis should give round 2
        assert_eq!(timestamp_to_round(QUICKNET_GENESIS_TIME + 3), 2);

        // 6 seconds after genesis should give round 3
        assert_eq!(timestamp_to_round(QUICKNET_GENESIS_TIME + 6), 3);
    }

    #[test]
    fn test_round_to_timestamp() {
        // Round 1 should give genesis time
        assert_eq!(round_to_timestamp(1), QUICKNET_GENESIS_TIME);

        // Round 2 should be 3 seconds after genesis
        assert_eq!(round_to_timestamp(2), QUICKNET_GENESIS_TIME + 3);
    }

    #[test]
    fn test_round_conversion_roundtrip() {
        let original_round = 1000000u64;
        let timestamp = round_to_timestamp(original_round);
        let recovered_round = timestamp_to_round(timestamp);
        assert_eq!(original_round, recovered_round);
    }

    #[test]
    fn test_datetime_to_round() {
        use chrono::TimeZone;

        // Create a datetime after genesis
        let dt = Utc.timestamp_opt(QUICKNET_GENESIS_TIME as i64 + 10, 0).unwrap();
        let round = datetime_to_round(dt);

        // Should be round 4 + 1 (for safety margin) = 5
        // (10 seconds / 3 second period) + 1 = 4, then +1 for margin = 5
        assert!(round >= 4);
    }

    // Note: Integration tests for encrypt/decrypt require network access
    // and a future unlock time that has passed. These should be run
    // as integration tests with appropriate timeouts.

    #[test]
    #[ignore] // Requires network access
    fn test_encrypt_decrypt_past_time() {
        use chrono::Duration;

        let password = "test_secret_password";
        // Use a time in the past (already unlockable)
        let unlock_time = Utc::now() - Duration::minutes(5);

        let encrypted = encrypt_with_tlock(password, unlock_time)
            .expect("Encryption should succeed");

        let decrypted = decrypt_with_tlock(&encrypted, unlock_time)
            .expect("Decryption should succeed for past time");

        assert_eq!(password, decrypted);
    }

    #[test]
    fn test_encrypt_creates_valid_output() {
        use chrono::Duration;

        let password = "test_password";
        let unlock_time = Utc::now() + Duration::hours(1);

        let encrypted = encrypt_with_tlock(password, unlock_time)
            .expect("Encryption should succeed");

        // Should be valid base64
        let decoded = BASE64.decode(&encrypted)
            .expect("Should be valid base64");

        // Should have at least 8 bytes for round + some ciphertext
        assert!(decoded.len() > 8);

        // First 8 bytes should be a valid round number
        let round_bytes: [u8; 8] = decoded[0..8].try_into().unwrap();
        let round = u64::from_be_bytes(round_bytes);
        assert!(round > 0);
    }

    #[test]
    fn test_get_tlock_info() {
        use chrono::Duration;

        let password = "test";
        let unlock_time = Utc::now() + Duration::hours(24);

        let encrypted = encrypt_with_tlock(password, unlock_time)
            .expect("Encryption should succeed");

        let (round, unlock_ts, available) = get_tlock_info(&encrypted)
            .expect("Should extract info");

        assert!(round > 0);
        assert!(unlock_ts > 0);
        // Should not be available yet (24 hours in future)
        assert!(!available);
    }

    #[test]
    fn test_decrypt_future_time_fails() {
        use chrono::Duration;

        let password = "secret";
        let unlock_time = Utc::now() + Duration::hours(24);

        let encrypted = encrypt_with_tlock(password, unlock_time)
            .expect("Encryption should succeed");

        // Attempting to decrypt should fail with TimeLockActive
        let result = decrypt_with_tlock(&encrypted, unlock_time);
        assert!(matches!(result, Err(TimeLockerError::TimeLockActive)));
    }
}
