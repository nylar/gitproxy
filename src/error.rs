use connectrpc::ConnectError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git error {0}")]
    Git(#[from] git2::Error),
    #[error("IO error {0}")]
    IO(#[from] std::io::Error),
    #[error("Axum error {0}")]
    Axum(#[from] axum::Error),
    #[error("Reading environment {0}")]
    Envy(#[from] envy::Error),
    #[error("reference {0} is not a valid commit")]
    InvalidCommit(String),
    #[error("branch {0} already exists")]
    BranchExists(String),
    #[error("Tokio task error {0}")]
    TokioTask(#[from] tokio::task::JoinError),
}

impl From<Error> for ConnectError {
    fn from(err: Error) -> Self {
        tracing::error!(error = err.to_string());
        match err {
            Error::Git(error) => ConnectError::internal(error.to_string()),
            Error::IO(error) => ConnectError::internal(error.to_string()),
            Error::Axum(error) => ConnectError::internal(error.to_string()),
            Error::Envy(error) => ConnectError::internal(error.to_string()),
            Error::InvalidCommit(error) => ConnectError::invalid_argument(error),
            Error::BranchExists(error) => ConnectError::already_exists(error),
            Error::TokioTask(error) => ConnectError::internal(error.to_string()),
        }
    }
}
