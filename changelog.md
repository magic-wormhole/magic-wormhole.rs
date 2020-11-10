# Changelog

## Unreleased changes

- Added API documentation \o/ (still a long way to go though)
- Reworked Key API. It now uses type-level programming to distinguish key purposes, in the hope you'll never ever confuse them.
- Internal improvements in Transit implementation. Little API changed except for the Keys.

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
