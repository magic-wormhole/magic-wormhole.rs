#![forbid(unsafe_code)]
#![deny(warnings)]
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
extern crate hex;
extern crate hkdf;
extern crate rand;
extern crate regex;
extern crate rustc_serialize;
extern crate sha2;
extern crate sodiumoxide;
extern crate spake2;

pub mod core;
pub mod io;

#[cfg(feature = "io_blocking")]
extern crate url;
#[cfg(feature = "io_blocking")]
extern crate ws;

#[cfg(feature = "io_tokio")]
extern crate futures;
#[cfg(feature = "io_tokio")]
extern crate tokio_core;
#[cfg(feature = "io_tokio")]
extern crate websocket;
