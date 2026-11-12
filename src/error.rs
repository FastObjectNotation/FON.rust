use std::fmt;


/// Errors returned by this crate. Each variant carries a message.
#[derive(Debug)]
#[non_exhaustive]
pub enum FonError {
    /// The text is not valid FON. For dump input the message starts with the 1-based line
    /// number, for example `line 12: Failed to parse number: 'x'`.
    Parse(String),
    /// A file could not be written.
    Write(String),
    /// An argument was rejected, for example a reserved character in a key.
    InvalidArgument(String),
    /// A file could not be opened or read.
    Io(String),
}


impl fmt::Display for FonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FonError::Parse(m) => write!(f, "{}", m),
            FonError::Write(m) => write!(f, "{}", m),
            FonError::InvalidArgument(m) => write!(f, "{}", m),
            FonError::Io(m) => write!(f, "{}", m),
        }
    }
}


impl std::error::Error for FonError {}
