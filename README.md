# Rusty Wormhole

Get things from one computer to another, safely.

<http://magic-wormhole.io/>

This is a Rust port of the Python version at <https://github.com/magic-wormhole/magic-wormhole>.

## Comparison with the Python implementation

Features that are missing:

- Tab completion
- Text message sending
- Folder sending (we can send folders, but it will send a tar ball which the other side will have to manually unpack)
- Tor support

New features that exceed the other implementations:

- Can do direct connections across the internet and firewalls
- Port forwarding in addition to file transfer (experimental)
- Send a file to multiple people (experimental)

## Getting started

If you want to toy with the CLI, `cargo run -- --help` will get you started. The code sits in `./cli/src/bin`.

If you'd like to use Wormhole in your application, `cargo doc --open` will tell you how to use it. There aren't any hosted docs at the moment.

If you don't fear touching code and want to contribute, `./src/lib.rs`, `./src/transfer.rs` and `./src/transit.rs` are rather easy to get into. The [protocol specification](https://github.com/magic-wormhole/magic-wormhole-protocols) will probably be useful to you.

## License

This work is licensed under the EUPL v1.2 or later. Contact the owner(s) for use in proprietary software.

----------

[![Matrix][matrix-room-image]][matrix-room-url]
[![Irc][irc-room-image]][irc-room-url]
![Build Status][build-status-image]
[![Deps][deps-status-image]][deps-status-url]
[![Codecov][codecov-image]][codecov-url]
[![Is-It-Maintained-Resolution-Time][iim-resolution-image]][iim-resolution-url]
[![Crates.io][crates-io-image]][crates-io-url]
[![Docs.rs][docs-image]][docs-url]

[matrix-room-image]: https://img.shields.io/badge/matrix.org-%23magic--wormhole-brightgreen
[matrix-room-url]: https://matrix.to/#/#magic-wormhole:matrix.org
[irc-room-image]: https://img.shields.io/badge/irc.libera.chat-%23magic--wormhole-brightgreen
[irc-room-url]: https://web.libera.chat/
[build-status-image]: https://github.com/magic-wormhole/magic-wormhole.rs/workflows/Rust/badge.svg
[deps-status-image]: https://deps.rs/repo/github/magic-wormhole/magic-wormhole.rs/status.svg
[deps-status-url]: https://deps.rs/repo/github/magic-wormhole/magic-wormhole.rs
[codecov-image]: https://codecov.io/gh/magic-wormhole/magic-wormhole.rs/branch/master/graph/badge.svg
[codecov-url]: https://codecov.io/gh/magic-wormhole/magic-wormhole.rs
[crates-io-image]: https://img.shields.io/crates/v/magic-wormhole.svg
[crates-io-url]: https://crates.io/crates/magic-wormhole
[docs-image]: https://docs.rs/magic-wormhole/badge.svg
[docs-url]: https://docs.rs/magic-wormhole
[iim-resolution-image]: http://isitmaintained.com/badge/resolution/magic-wormhole/magic-wormhole.rs.svg
[iim-resolution-url]: http://isitmaintained.com/project/magic-wormhole/magic-wormhole.rs
