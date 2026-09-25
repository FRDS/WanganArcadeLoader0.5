//! Everything that differs between Linux (LD_PRELOAD) and Windows (a LINE
//! plugin). The rest of the crate goes through these functions.

pub mod call;
pub use call::call_game;

#[cfg(unix)]
mod linux;
#[cfg(unix)]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
