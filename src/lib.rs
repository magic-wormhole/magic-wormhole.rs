mod core;

#[cfg(feature = "io_blocking")]
pub mod io::blocking;
#[cfg(feature = "io_tokio")]
pub mod io::tokio;
