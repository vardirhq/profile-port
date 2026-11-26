use crate::code::CodeError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error writing or reading the package: {0}")]
    Io(#[from] std::io::Error),
    /// Reading the *source* file failed (as opposed to the package). The
    /// writer has already recorded the file as aborted; the caller decides
    /// how to report it.
    #[error("could not read source file: {0}")]
    SourceRead(std::io::Error),
    #[error("this folder is not a ProfilePort package")]
    NotAPackage,
    #[error("this package was made by a newer ProfilePort (format {0})")]
    UnsupportedVersion(u16),
    #[error(transparent)]
    Code(#[from] CodeError),
    #[error("that transfer code does not unlock this package")]
    WrongCode,
    #[error("package data is damaged or has been altered: {0}")]
    Corrupt(String),
    #[error("unsafe path in package: {0:?}")]
    UnsafePath(String),
    #[error("this package is already complete")]
    AlreadyComplete,
    #[error("the package already holds backed-up data or other files, so it was not deleted")]
    NotDiscardable,
    #[error("invalid ticket number {0:?}: use 1–32 letters, digits, '-' or '_'")]
    InvalidTicket(String),
}

pub type Result<T> = std::result::Result<T, Error>;
