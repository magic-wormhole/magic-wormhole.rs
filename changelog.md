# Changelog

## Unreleased

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
