use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HerdrError {
    Socket(String),
    Server { code: String, message: String },
    Protocol(String),
    Closed,
    /// A read ran past the timeout once the request was written: it may still take effect (hc may hold keys).
    Timeout(String),
    /// A key gate said no: the input stopped there; the keys after it were not sent (Swift `KeyGateStop`).
    KeyGateStop,
    /// Swift `QuestionAdapter.Failure`.
    Question(QuestionFailure),
}

/// Why a question could not be answered with keys (Swift `QuestionAdapter.Failure`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionFailure {
    /// The pane does not show this question's picker: nothing was sent.
    NotOnScreen,
    /// This answer cannot be given with keys. Nothing was sent.
    Unsupported(String),
    /// Keys were sent, but the expected screen did not come (stopped there).
    Stuck(String),
    /// All keys were sent, but the picker is still on screen.
    StillOnScreen,
}

impl fmt::Display for QuestionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuestionFailure::NotOnScreen => write!(f, "the question is not on screen"),
            QuestionFailure::Unsupported(m) => write!(f, "cannot answer: {m}"),
            QuestionFailure::Stuck(m) => write!(f, "stopped waiting for: {m}"),
            QuestionFailure::StillOnScreen => write!(f, "the question is still on screen after answering"),
        }
    }
}

impl fmt::Display for HerdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HerdrError::Socket(m) => write!(f, "socket: {m}"),
            HerdrError::Server { code, message } => write!(f, "server error {code}: {message}"),
            HerdrError::Protocol(m) => write!(f, "protocol: {m}"),
            HerdrError::Closed => write!(f, "connection closed"),
            HerdrError::Timeout(m) => write!(f, "timeout: {m}"),
            HerdrError::KeyGateStop => write!(f, "stopped: the pane, its agent or this device's input level changed"),
            HerdrError::Question(q) => write!(f, "{q}"),
        }
    }
}

impl std::error::Error for HerdrError {}
