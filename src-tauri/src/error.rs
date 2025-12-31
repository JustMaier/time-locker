use thiserror::Error;

#[derive(Error, Debug)]
pub enum TimeLockerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Archive error: {0}")]
    Archive(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid key file format")]
    InvalidKeyFile,

    #[error("Time lock not yet expired")]
    TimeLockActive,

    #[error("Network error: {0}")]
    Network(String),

    #[error("Drand beacon unavailable: {0}")]
    DrandUnavailable(String),

    #[error("Command execution error: {0}")]
    CommandExecution(String),

    #[error("YAML parsing error: {0}")]
    YamlParse(String),

    #[error("Date/Time parsing error: {0}")]
    DateTimeParse(#[from] chrono::ParseError),

    #[error("Missing field: {0}")]
    MissingField(String),

    #[error("File not found: {0}")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, TimeLockerError>;
