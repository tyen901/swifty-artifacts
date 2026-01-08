//! .NET DateTime ticks utilities.

use std::time::{SystemTime, UNIX_EPOCH};

pub const DOTNET_EPOCH_TICKS: u64 = 621_355_968_000_000_000;
pub const TICKS_PER_SEC: u64 = 10_000_000;

/// Compute current UTC .NET DateTime ticks (100ns since 0001-01-01 UTC).
pub fn dotnet_ticks_from_system_time() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch");

    DOTNET_EPOCH_TICKS + now.as_secs() * TICKS_PER_SEC + u64::from(now.subsec_nanos()) / 100
}
