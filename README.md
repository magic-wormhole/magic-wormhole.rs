Magic Wormhole (rust edition)

* Get things from one computer to another, safely.

http://magic-wormhole.io/

This is a Rust port of the Python version at https://github.com/warner/magic-wormhole .

It is a work in progress: much of the low-level protocol is implemented, but
not the file-transfer part. As of version 0.0.1, `wormhole send --text
MESSAGE` can talk to `wormhole receive CODE`, as long as you get the code
right. If you get the code wrong, the clients hang instead of showing an
error message. Interactive code input (i.e. `wormhole receive`, without the
code argument) does not work, nor does sending files (i.e. `wormhole send`
without the `--text=` argument). These will come eventually.


[![Build Status][build-status-image]][build-status-url]
[![CircleCI Status][circleci-status-image]][circleci-status-url]
[![Deps][deps-status-image]][deps-status-url]
[![Codecov][codecov-image]][codecov-url]
[![Is-It-Maintained-Resolution-Time][iim-resolution-image]][iim-resolution-url]
[![Is-It-Maintained-Open-Issues][iim-open-image]][iim-open-url]
[![Crates.io][crates-io-image]][crates-io-url]
[![Docs.rs][docs-image]][docs-url]
[![License][license-image]][license-url]

[build-status-image]: https://travis-ci.org/warner/magic-wormhole.rs.svg?branch=master
[build-status-url]: https://travis-ci.org/warner/magic-wormhole.rs
[circleci-status-image]: https://circleci.com/gh/warner/magic-wormhole.rs.svg?style=svg
[circleci-status-url]: https://circleci.com/gh/warner/magic-wormhole.rs
[deps-status-image]: https://deps.rs/repo/github/warner/magic-wormhole.rs/status.svg
[deps-status-url]: https://deps.rs/repo/github/warner/magic-wormhole.rs
[codecov-image]: https://codecov.io/gh/warner/magic-wormhole.rs/branch/master/graph/badge.svg
[codecov-url]: https://codecov.io/gh/warner/magic-wormhole.rs
[crates-io-image]: https://img.shields.io/crates/v/magic-wormhole.svg
[crates-io-url]: https://crates.io/crates/magic-wormhole
[docs-image]: https://docs.rs/magic-wormhole/badge.svg
[docs-url]: https://docs.rs/magic-wormhole
[license-image]: https://img.shields.io/crates/l/magic-wormhole.svg
[license-url]: LICENSE
[iim-resolution-image]: http://isitmaintained.com/badge/resolution/warner/magic-wormhole.rs.svg
[iim-resolution-url]: http://isitmaintained.com/project/warner/magic-wormhole.rs
[iim-open-image]: http://isitmaintained.com/badge/open/warner/magic-wormhole.rs.svg
[iim-open-url]: http://isitmaintained.com/project/warner/magic-wormhole.rs
