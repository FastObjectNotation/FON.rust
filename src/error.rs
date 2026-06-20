use std::fmt;


#[derive(Debug)]
pub enum FonError {
    Parse(String),
    Write(String),
    InvalidArgument(String),
}


impl fmt::Display for FonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FonError::Parse(m) => write!(f, "{}", m),
            FonError::Write(m) => write!(f, "{}", m),
            FonError::InvalidArgument(m) => write!(f, "{}", m),
        }
    }
}


impl std::error::Error for FonError {}
