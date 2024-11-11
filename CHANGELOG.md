# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.4] - 2024-11-23

### Fixed

- cli: autocomplete would use the wrong wordlist exactly 100% of the time 🙈
- cli: Remove unmaintained instant dependency

## [0.7.3] - 2024-10-25

### Added

- cli: Add clipboard auto completion support

## [0.7.2] - 2024-10-11

### Changed

- \[all\]\[\breaking\] Code words with a secret password section shorter than 4 bytes are no longer accepted. This only breaks completely invalid uses of the code.
- \[all\] Code words with a weak password section or a non-integer nameplate will throw an error in the long. This error can be upgraded to a hard error by enabling the "entropy" feature. This feature will become the default in the next major release.
- \[lib\] Implemented FromStr for `Code` and `Nameplate`
- \[lib\] Added new checked type for the `Password` section of a wormhole code
- \[lib\] Added new `entropy` feature. When enabled, the entropy of the passed password will be checked on creation. This will change the signature of `MailboxConnection::create_with_password` to require the password to be passed via the new `Password` wrapper type.
- \[lib\]\[deprecated\] Deprecated the `Code` and `Nameplate` `From<impl Into<String>>` implementations and `new()` methods. They are unchecked and will print a warning for now. These will be removed in the next breaking release.

## [0.7.1] - 2024-07-25

### Changed

- Bump dependencies

### Fixed

- [openssl's `MemBio::get_buf` has undefined behavior with empty buffers](https://github.com/advisories/GHSA-q445-7m23-qrmw)

## [0.7.0] - 2024-07-17

### Changed

- \[all\]\[breaking\] By default websocket TLS support is now disabled in the library crate. TLS is required for secure websocket connections to the mailbox server (`wss://`). As the handshake protocol itself is encrypted, this extra layer of encryption is superfluous. Most WASM targets however refuse to connect to non-TLS websockets. For maximum compatibility with all mailbox servers, or for web browser support, select a TLS implementation by specifying the feature flag `tls` for a statically linked implementation via the `ring` crate, or `native-tls` for dynamically linking with the system-native TLS implementation.
- \[all\] For experimental (unstable) `transfer-v2` protocol support, enable feature flag `experimental-transfer-v2`. The protocol is not yet finalized.
- \[all\] Added compilation support for WASM targets.
- \[lib\]\[breaking\] replaced `transit::TransitInfo` with a struct containing the address and a `conn_type` field which contains the old enum as `transit::ConnectionType`
- \[lib\]\[breaking\] changed the signature of the `transit_handler` function to take just the newly combined `transit::TransitInfo`
- \[lib\]\[breaking\] changed the signature of the `file_name` argument to `transfer::send_*` to take `Into<String>` instead of `Into<PathBuf>`
- \[lib\]\[breaking\] replaced `transfer::AppVersion` with a struct with private fields that implements `std::default::Default`
- \[lib\]\[deprecated\] split `Wormhole` in `MailboxConnection` and `Wormhole`
- \[lib\]\[deprecated\] `Wormhole::connect_with(out)_code`, `WormholeWelcome`, use `MailboxConnection::create()` and then `Wormhole::connect()` instead
- \[lib\]\[deprecated\] `Wormhole` public struct fields. Use the provided accessor methods instead.
- \[lib\]\[deprecated\] `ReceiveRequest.filename` is deprecated and replaced by `ReceiveRequest.file_name(..)`
- \[lib\]\[deprecated\] `ReceiveRequest.filesize` is deprecated and replaced by `ReceiveRequest.file_size(..)`
- \[lib\]\[deprecated\] `GenericKey`, implement `KeyPurpose` on a custom struct instead
- \[lib\]\[deprecated\] `rendezvous::RendezvousServer` will be removed in the future with no planned public replacement.
- \[lib\]\[deprecated\] `transfer::PeerMessage` will be removed in the future with no planned public replacement.
- \[lib\]\[deprecated\] `transit::TransitConnector` will be removed in the future with no planned public replacement.
- \[lib\]\[deprecated\] `transit::log_transit_connection` and implemented `Display` on `TransitInfo` instead.
- \[lib\]\[deprecated\] `transit::init()` will be removed in the future with no planned public replacement.

## [0.6.1] - 2023-12-03

### Fixed

- RUSTSEC-2023-0065: Update tungstenite
- RUSTSEC-2023-0037: Replace xsalsa20poly1305 with crypto_secretbox
- RUSTSEC-2023-0052: Update webpki

### Changed

- Update crate dependencies

## [0.6.0] - 2022-12-21

### Added

- Add shell completion support for the CLI
- Add support for [wormhole URIs](https://github.com/magic-wormhole/magic-wormhole-protocols/pull/21)
	- \[cli\] The CLI will show a QR code (even if no app can probably read it currently) and a link
	- \[lib\] See `magic_wormhole::uri::WormholeTransferUri`

### Fixed

- Fix broken port forwarding
- Fix directory transfer
- Smaller bugfixes

### Changed

- \[lib\]\[breaking\] File transfer functions do not take a `url::Url` for the relay server anymore, but a `Vec<magic_wormhole::transit::RelayHint>`
	- For migration, look at `magic_wormhole::transit::RelayHint::from_urls`

## [0.5.0] - 2022-05-24

### Changed

- \[lib\]\[breaking\] Removed `relay-v2` ability again.
	- This fixed some relay signalling issues, where no connection could be made with `--force-relay` under some circumstances.
- \[lib\]\[breaking\] Exposed the state of the established transit connection
	- The `transit` module now returns whether the established connection is direct or not and the peer/relay's IP address
	- The `transfer` and `forwarding` modules now take a `transit_handler` argument. Use `&transit::log_transit_connection` as default value

### Fixed

- Various bugfixes

## [0.4.0] - 2022-03-23

### Added

- Added `--force-relay` and `--force-direct` CLI flags that control the transit connection
	- The feature is also exposed in the API

### Changed

- When sending, the code will now aumatically be copied into clipboard. So you don't have to select it in the terminal anymore before pasting!
- Updated a lot of dependencies
- Split the project into a workspace and feature gated some higher level protocols. This should now work way better on crates.io (and generally for library usage)

## [0.3.1] - 2022-03-22

### Changed

- yanked, changes moved to 0.4.0

## [0.3.0] - 2022-03-06

### Added

- Added experimental port forwarding feature

### Fixed

- Fixed `send-many` subcommand

### Changed

- Improved user experience with better logging and messages
- Improved error and cancellation handling
- Cleaned up CLI args and implemented previous placeholders
- Many internal refactorings to accomodate the changes. The public API did not change that much though.

## [0.2.0] - 2021-04-12

### Added

- Implemented version and verifier in the API
- Added API documentation \o/ (still a long way to go though)
- New features for file transfer
	- File acks are not sent automatically anymore, the receiver has to accept an offer now.
	- Existing files are not overwritten without permission

### Changed

- Reworked Key API. It now uses type-level programming to distinguish key purposes, in the hope you'll never ever confuse them.
- Internal improvements in Transit implementation. Little API changed except for the Keys.
- Internal rewrite of the `core`. This resulted in no public API changes except that the receiver is now `TryStream` instead of `Stream` (error handling, yay).
- Progress reporting support during transfers
- `send-many` got improved

## [0.1.0] - 2020-11-03

### Added

- Merged Transit/Transfer implementation from @vu3rdd and made it work.

### Changed

- Rewrote Wormhole API (and parts of Transit/Transfer as well)
	- Everything is async now (using `async_std`), there are no other (i.e. blocking) implementations.
	- The API exposed from `core` got flipped on its head too in the process.
- Moved IO layer into core; ported it from `ws` to `async-tungstenite`. Removed all `tokio` dependencies.
- Many other refactorings, thrown stuff around, in the hope of improving things.
	- Together with the changes noted above, the `io::*` modules got removed, as well as their Cargo feature flags (and dependencies).
	- There is only one feature flag left, it's for the binary.
- A bit of progress on the CLI side
	- Added an experimental `send-many` command. It will create a code and then simply send the file over and over again in a loop. Might be useful.

## [0.0.2] - 2019-09-01

### Changed

- No change log provided

## [0.0.1] - 2018-12-21

### Changed

- Initial release
