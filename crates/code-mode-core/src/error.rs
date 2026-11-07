use thiserror::Error;

/// Failure while performing one API operation from a script.
#[derive(Debug, Error)]
pub enum InvokeError {
    #[error("unknown operation `{0}`")]
    UnknownOperation(String),
    #[error("invalid arguments for `{operation}`: {message}")]
    InvalidArguments { operation: String, message: String },
    #[error("HTTP {status} calling `{operation}` {method} {path}: {body}")]
    Http {
        operation: String,
        method: String,
        path: String,
        status: u16,
        body: String,
    },
    #[error("transport error calling `{operation}`: {message}")]
    Transport { operation: String, message: String },
    #[error("call limit of {0} exceeded")]
    CallLimit(u32),
    #[error("cache error: {0}")]
    Cache(String),
}

/// Failure while evaluating a sandbox script.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("script is empty")]
    Empty,
    #[error("script is longer than {0} characters")]
    TooLarge(usize),
    #[error("script error: {0}")]
    Script(String),
}
