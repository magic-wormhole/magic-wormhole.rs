# Maintenance documentation

## Release

To create a new release, follow these steps:

- Update version number in the workspace Cargo.toml
- Update CHANGELOG.md with release version and date
- Update Cargo.lock
- Commit & push the changes
- Tag the commit: `git tag -as a.b.c`
- Push the tag: `git push origin a.b.c`
- Verify GitHub release was created by CI
- Push a new crate version to crates.io with `cargo publish -p magic-wormhole`
