# Magic Wormhole CLI

A rust implementation of the classic `magic-wormhole` command line client.

## Installation

### GitHub Releases

We publish source code and binaries to GitHub releases. Visit the [releases page](https://github.com/magic-wormhole/magic-wormhole.rs/releases) for the latest release.

Or use [cargo binstall](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall magic-wormhole-cli
```

### crates.io

You can use cargo to install the CLI from crates.io:

```bash
cargo install --locked magic-wormhole-cli
```

## Usage

```text
Get things from one computer to another, safely

Usage: wormhole-rs [OPTIONS] <COMMAND>

Commands:
  send       Send a file or a folder [aliases: tx]
  receive    Receive a file or a folder [aliases: rx]
  send-many  Send a file to many recipients
  forward    Forward ports from one machine to another

Options:
  -v, --verbose  Enable logging to stdout, for debugging purposes
  -h, --help     Print help
  -V, --version  Print version

Run a subcommand with `--help` to know how it's used.
To send files, use `wormhole send <PATH>`.
To receive files, use `wormhole receive <CODE>`.
```
