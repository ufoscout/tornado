use thiserror::Error;

pub mod actors;
pub mod pool;
pub mod wrapper;

#[derive(Error, Debug)]
pub enum TornadoError {
    #[error("SenderError: {message}")]
    SenderError { message: String },
    #[error("ActorCreationError: {message}")]
    ActorCreationError { message: String },
    #[error("ConfigurationError: {message}")]
    ConfigurationError { message: String },
}
