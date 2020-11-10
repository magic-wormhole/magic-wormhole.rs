# Rusty Wormhole

Get things from one computer to another, safely.

<http://magic-wormhole.io/>

This is a Rust port of the Python version at <https://github.com/warner/magic-wormhole>.

## This is a work in progress

As of version `0.1.0`, most of the major protocols are implemented. The library part can be deemed "usable", although the API is far away from being "stable". There is still a lot of work to be done here, notably error handling and code documentation.

The CLI is in a "proof of concept" state at the moment. Basic file sending and receiving is implemented, but everything else is missing. Note that this is not a 1:1 port of the Python CLI; and it won't be a drop-in replacement for it.

## Getting started

If you want to toy with the CLI, `cargo run -- --help` will get you started. The code sits in `./src/bin`.

If you'd like to use Wormhole in your application, `cargo doc --open` will tell you how to use it. There aren't any hosted docs at the moment.

If you don't fear touching code and want to contribute, `./src/lib.rs`, `./src/transfer.rs` and `./src/transit.rs` are rather easy to get into.

However before diving into the `core` module, you should definitively read the [spec](https://magic-wormhole.readthedocs.io/en/latest/) and the [implementation notes](https://github.com/piegamesde/magic-wormhole.rs/wiki) first. Maybe having a bit of understanding of the Python implementation doesn't hurt either.

----------

[![Matrix][matrix-room-image]][matrix-room-url]
<!-- ![Build Status][build-status-image] -->
<!-- [![CircleCI Status][circleci-status-image]][circleci-status-url] -->
[![Deps][deps-status-image]][deps-status-url]
<!-- [![Codecov][codecov-image]][codecov-url] -->
[![Is-It-Maintained-Resolution-Time][iim-resolution-image]][iim-resolution-url]
[![Is-It-Maintained-Open-Issues][iim-open-image]][iim-open-url]
<!-- [![Crates.io][crates-io-image]][crates-io-url] -->
<!-- [![Docs.rs][docs-image]][docs-url] -->
[![License][license-image]][license-url]

[matrix-room-image]: https://img.shields.io/matrix/rusty-wormhole:matrix.org
[matrix-room-url]: https://matrix.to/#/#rusty-wormhole:matrix.org
[build-status-image]: https://github.com/piegamesde/magic-wormhole.rs/workflows/Rust/badge.svg
[circleci-status-image]: https://circleci.com/gh/piegamesde/magic-wormhole.rs.svg?style=svg
[circleci-status-url]: https://circleci.com/gh/piegamesde/magic-wormhole.rs
[deps-status-image]: https://deps.rs/repo/github/piegamesde/magic-wormhole.rs/status.svg
[deps-status-url]: https://deps.rs/repo/github/piegamesde/magic-wormhole.rs
[codecov-image]: https://codecov.io/gh/piegamesde/magic-wormhole.rs/branch/master/graph/badge.svg
[codecov-url]: https://codecov.io/gh/piegamesde/magic-wormhole.rs
[crates-io-image]: https://img.shields.io/crates/v/magic-wormhole.svg
[crates-io-url]: https://crates.io/crates/magic-wormhole
[docs-image]: https://docs.rs/magic-wormhole/badge.svg
[docs-url]: https://docs.rs/magic-wormhole
[license-image]: https://img.shields.io/crates/l/magic-wormhole.svg
[license-url]: LICENSE
[iim-resolution-image]: http://isitmaintained.com/badge/resolution/piegamesde/magic-wormhole.rs.svg
[iim-resolution-url]: http://isitmaintained.com/project/piegamesde/magic-wormhole.rs
[iim-open-image]: http://isitmaintained.com/badge/open/piegamesde/magic-wormhole.rs.svg
[iim-open-url]: http://isitmaintained.com/project/piegamesde/magic-wormhole.rs
