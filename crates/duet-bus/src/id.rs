//! A fresh entry's id and time.

use std::time::{SystemTime, UNIX_EPOCH};

use duet_schemas::journal::{At, Id};

use crate::{Error, Result};

fn unix_ms() -> u64 {
    // Before 1970 is no time a journal is written at: it reads as 1970.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Now, to the millisecond.
#[must_use]
pub fn now() -> At {
    At::from_unix_ms(unix_ms())
}

/// A fresh UUIDv7: now, and 74 bits from the operating system's randomness.
///
/// # Errors
///
/// [`Error::Random`] when the operating system gives none.
pub fn fresh_id() -> Result<Id> {
    let mut random = [0u8; 10];
    getrandom::fill(&mut random).map_err(|e| Error::Random(e.to_string()))?;
    Ok(Id::from_parts(unix_ms(), random))
}
