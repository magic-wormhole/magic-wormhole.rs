use events::Phase;

#[test]
fn test_phase() {
    let p = Phase("pake".to_string());
    assert_eq!(p.to_string(), "pake"); // Order looks for "pake"
}
