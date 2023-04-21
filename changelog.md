# Changelog

## Unreleased

- Added compilation support for WASM targets.
- \[lib\]\[breaking\] replaced `transit::TransitInfo` with a struct containing the address, the old enum has been renamed to `transit::ConnectionType`.

## Version 0.6.0

- Add shell completion support for the CLI
- Add support for [wormhole URIs](https://github.com/magic-wormhole/magic-wormhole-protocols/pull/21)
	- \[cli\] The CLI will show a QR code (even if no app can probably read it currently) and a link
	- \[lib\] See `magic_wormhole::uri::WormholeTransferUri`
- \[lib\]\[breaking\] File transfer functions do not take a `url::Url` for the relay server anymore, but a `Vec<magic_wormhole::transit::RelayHint>`
	- For migration, look at `magic_wormhole::transit::RelayHint::from_urls`
- Fix broken port forwarding
- Fix directory transfer
- Smaller bugfixes

## Version 0.5.0

- \[lib\]\[breaking\] Removed `relay-v2` ability again.
	- This fixed some relay signalling issues, where no connection could be made with `--force-relay` under some circumstances.
- \[lib\]\[breaking\] Exposed the state of the established transit connection
	- The `transit` module now returns whether the established connection is direct or not and the peer/relay's IP address
	- The `transfer` and `forwarding` modules now take a `transit_handler` argument. Use `&transit::log_transit_connection` as default value
- Various bugfixes

## Version 0.4.0

- When sending, the code will now aumatically be copied into clipboard. So you don't have to select it in the terminal anymore before pasting!
- Added `--force-relay` and `--force-direct` CLI flags that control the transit connection
	- The feature is also exposed in the API
- Updated a lot of dependencies
- Split the project into a workspace and feature gated some higher level protocols. This should now work way better on crates.io (and generally for library usage)

## Version 0.3.1

*yanked, changes moved to 0.4.0*

## Version 0.3.0

- Added experimental port forwarding feature
- Improved user experience with better logging and messages
- Improved error and cancellation handling
- Cleaned up CLI args and implemented previous placeholders
- Fixed `send-many` subcommand
- Many internal refactorings to accomodate the changes. The public API did not change that much though.

## Version 0.2.0

- Implemented version and verifier in the API
- Added API documentation \o/ (still a long way to go though)
- Reworked Key API. It now uses type-level programming to distinguish key purposes, in the hope you'll never ever confuse them.
- New features for file transfer
	- File acks are not sent automatically anymore, the receiver has to accept an offer now.
	- Existing files are not overwritten without permission
- Internal improvements in Transit implementation. Little API changed except for the Keys.
- Internal rewrite of the `core`. This resulted in no public API changes except that the receiver is now `TryStream` instead of `Stream` (error handling, yay).
- Progress reporting support during transfers
- `send-many` got improved

## Version 0.1.0

- Merged Transit/Transfer implementation from @vu3rdd and made it work.
- Rewrote Wormhole API (and parts of Transit/Transfer as well)
	- Everything is async now (using `async_std`), there are no other (i.e. blocking) implementations.
	- The API exposed from `core` got flipped on its head too in the process.
- Moved IO layer into core; ported it from `ws` to `async-tungstenite`. Removed all `tokio` dependencies.
- Many other refactorings, thrown stuff around, in the hope of improving things.
	- Together with the changes noted above, the `io::*` modules got removed, as well as their Cargo feature flags (and dependencies).
	- There is only one feature flag left, it's for the binary.
- A bit of progress on the CLI side
	- Added an experimental `send-many` command. It will create a code and then simply send the file over and over again in a loop. Might be useful.

## Older versions

- 0.0.3 (never released): No change log provided
- 0.0.2: No change log provided
- 0.0.1: Initial release
