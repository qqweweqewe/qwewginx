use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Msg(String),
}
