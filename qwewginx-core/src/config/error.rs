use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse: {0}")]
    Parse(#[from] Box<pest::error::Error<crate::config::parser::Rule>>),

    #[error("{0}")]
    Msg(String),
}
