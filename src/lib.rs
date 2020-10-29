#![forbid(unsafe_code)]
// #![deny(warnings)]

pub mod core;
pub mod io;

pub use crate::io::blocking::connect_1;
