#![forbid(unsafe_code)]
#![deny(warnings)]

use hex;
use sodiumoxide;

pub mod core;
pub mod io;

#[cfg(feature = "io-blocking")]
extern crate ws;
