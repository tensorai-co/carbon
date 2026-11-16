//! Carbon's errors: a kind to match on, and a message that says what went wrong in words.

use std::fmt;

/// What kind of thing went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// No machine or image by that name, or a program that isn't on the machine's disk.
    NotFound,
    /// A machine by that name is there already.
    AlreadyExists,
    /// The machine is running, and this needs it stopped.
    Running,
    /// The machine isn't running, and this needs it to be.
    NotRunning,
    /// Something asked for makes no sense: a bad size, platform, port or argument.
    InvalidInput,
    /// Not there on this host yet, or not for this kernel.
    Unsupported,
    /// The machine's hooks or rules refused it.
    Denied,
    /// Pulling, importing or reading an image.
    Image,
    /// Reading or writing the host's files, or talking to a running machine.
    Io,
    /// The machine couldn't be made or run: no backend, a kernel that couldn't boot, a host port in use.
    Failed,
}

/// An error from Carbon: its kind, and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into() }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::NotFound, message)
    }
    pub(crate) fn invalid(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::InvalidInput, message)
    }
    pub(crate) fn unsupported(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Unsupported, message)
    }
    pub(crate) fn io(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Io, message)
    }
    pub(crate) fn no_machine(name: &str) -> Error {
        Error::not_found(format!("there's no machine called {name}"))
    }
    pub(crate) fn not_running(name: &str) -> Error {
        Error::new(ErrorKind::NotRunning, format!("{name} isn't running"))
    }
    pub(crate) fn running(name: &str) -> Error {
        Error::new(ErrorKind::Running, format!("{name} is running: stop it first"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The parts below `carbon` say what went wrong as text: a machine that couldn't be made or run.
impl From<String> for Error {
    fn from(message: String) -> Error {
        Error::new(ErrorKind::Failed, message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Error {
        Error::new(ErrorKind::Failed, message)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::new(ErrorKind::Io, e.to_string())
    }
}

/// Carbon's results.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An image's errors, which come as text, as image errors.
pub(crate) fn image(message: String) -> Error {
    Error::new(ErrorKind::Image, message)
}
