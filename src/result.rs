use communication::Command;
use httparse;
use mio;
use std::borrow::Cow;
use std::convert::{From, Into};
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::result::Result as StdResult;
use std::str::Utf8Error;

pub type Result<T> = StdResult<T, Error>;

#[derive(Debug)]
pub enum Kind {
    Internal,
    Capacity,
    Protocol,
    Encoding(Utf8Error),
    Io(io::Error),
    Http(httparse::Error),
    Queue(mio::channel::SendError<Command>),
    /// Indicates a failure to schedule a timeout on the EventLoop.
    Timer(mio::timer::TimerError),
    Custom(Box<StdError + Send + Sync>),
}

/// A struct indicating the kind of error that has occured and any precise details of that error.
pub struct Error {
    pub kind: Kind,
    pub details: Cow<'static, str>,
}

impl Error {
    pub fn new<I>(kind: Kind, details: I) -> Error
    where
        I: Into<Cow<'static, str>>,
    {
        Error {
            kind: kind,
            details: details.into(),
        }
    }

    pub fn into_box(self) -> Box<StdError> {
        match self.kind {
            Kind::Custom(err) => err,
            _ => Box::new(self),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.details.len() > 0 {
            write!(f, "Error <{:?}>: {}", self.kind, self.details)
        } else {
            write!(f, "Error <{:?}>", self.kind)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.details.len() > 0 {
            write!(f, "{}: {}", self.description(), self.details)
        } else {
            write!(f, "{}", self.description())
        }
    }
}

impl StdError for Error {
    fn description(&self) -> &str {
        match self.kind {
            Kind::Internal => "Internal Application Error",
            Kind::Capacity => "Socket at Capacity",
            Kind::Protocol => "Socket Protocol Error",
            Kind::Encoding(ref err) => err.description(),
            Kind::Io(ref err) => err.description(),
            Kind::Http(_) => "Unable to parse HTTP",
            Kind::Queue(_) => "Unable to send signal on event loop",
            Kind::Timer(_) => "Unable to schedule timeout on event loop",
            Kind::Custom(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&StdError> {
        match self.kind {
            Kind::Encoding(ref err) => Some(err),
            Kind::Io(ref err) => Some(err),
            Kind::Custom(ref err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        let detail = err.to_string();
        Error::new(Kind::Io(err), detail)
    }
}

impl From<httparse::Error> for Error {
    fn from(err: httparse::Error) -> Error {
        let details = match err {
            httparse::Error::HeaderName => "Invalid byte in header name.",
            httparse::Error::HeaderValue => "Invalid byte in header value.",
            httparse::Error::NewLine => "Invalid byte in new line.",
            httparse::Error::Status => "Invalid byte in Response status.",
            httparse::Error::Token => "Invalid byte where token is required.",
            httparse::Error::TooManyHeaders => {
                "Parsed more headers than provided buffer can contain."
            }
            httparse::Error::Version => "Invalid byte in HTTP version.",
        };

        Error::new(Kind::Http(err), details)
    }
}

impl From<mio::channel::SendError<Command>> for Error {
    fn from(err: mio::channel::SendError<Command>) -> Error {
        match err {
            mio::channel::SendError::Io(err) => {
                let detail = err.to_string();
                Error::new(Kind::Io(err), detail)
            }
            _ => {
                let detail = err.to_string();
                Error::new(Kind::Queue(err), detail)
            }
        }
    }
}

impl From<mio::timer::TimerError> for Error {
    fn from(err: mio::timer::TimerError) -> Error {
        match err {
            _ => {
                let detail = err.to_string();
                Error::new(Kind::Timer(err), detail)
            }
        }
    }
}

impl From<Utf8Error> for Error {
    fn from(err: Utf8Error) -> Error {
        let detail = err.to_string();
        Error::new(Kind::Encoding(err), detail)
    }
}


impl<B> From<Box<B>> for Error
where
    B: StdError + Send + Sync + 'static,
{
    fn from(err: Box<B>) -> Error {
        let detail = err.to_string();
        Error::new(Kind::Custom(err), detail)
    }
}
