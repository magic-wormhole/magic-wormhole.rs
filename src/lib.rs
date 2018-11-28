#![forbid(unsafe_code)]
#![deny(warnings)]

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
use hex;

use sodiumoxide;

pub mod core;
pub mod io;

#[cfg(feature = "io-blocking")]
extern crate ws;
