//! Integration tests for the wormhole cli

#[test]
fn trycmd() {
    trycmd::TestCases::new()
        .case("tests/cmd/*.trycmd")
        .case("tests/cmd/*.toml");
}
