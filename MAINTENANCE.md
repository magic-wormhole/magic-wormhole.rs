# Maintenance documentation

## Release

To create a new release, follow these steps:

- Update version number in Cargo.toml for library and CLI
- Update Cargo.lock
- Commit the changes
- Tag the commit: `git tag -as a.b.c`
- Push the tag: `git push origin a.b.c`
- Create a github release for the tag and upload the built binaries from the github actions workflow
- Push a new crate version to crates.io with `cargo publish -p magic-wormhole`
