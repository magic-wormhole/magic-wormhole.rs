use api::Mood;
use events::Phase;

#[test]
fn test_phase() {
    let p = Phase("pake".to_string());
    assert_eq!(p.to_string(), "pake"); // Order looks for "pake"
}

#[test]
fn test_mood() {
    // these strings are part of the wire protocol, so .to_string() must
    // return exactly these values, even if i18n or other human-facing
    // concerns want it otherwise. If we must change our implementation of
    // the Display trait, then we need to add a different trait specifically
    // for serialization onto the wire. They must match the strings used in
    // the Python version in src/wormhole/_boss.py , in calls to
    // self._T.close()
    assert_eq!(Mood::Happy.to_string(), "happy");
    assert_eq!(Mood::Lonely.to_string(), "lonely");
    assert_eq!(Mood::Error.to_string(), "errory");
    assert_eq!(Mood::Scared.to_string(), "scary");
    assert_eq!(Mood::Unwelcome.to_string(), "unwelcome");
}
