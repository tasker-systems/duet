/// Everything that can go wrong capturing a take.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A source, a stage or a conversion met input it cannot work with.
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
