use serde_derive::Serialize;
use serde_json::{json, Value};
use std::time::SystemTime;

#[derive(Debug, PartialEq, Serialize)]
pub struct TimingLogEvent {
    name: String,
    start: Option<f64>,
    stop: Option<f64>,
    details: Value,
}

pub fn now() -> f64 {
    let dur =
        SystemTime::duration_since(&SystemTime::now(), SystemTime::UNIX_EPOCH)
            .unwrap();
    //dur.as_float_secs()
    let secs = dur.as_secs() as f64;
    let nanos = f64::from(dur.subsec_nanos()) * 0.000_000_001;
    secs + nanos
}

pub fn new_timelog(name: &str, when: Option<f64>) -> TimingLogEvent {
    TimingLogEvent {
        name: name.to_string(),
        start: match when {
            Some(_) => when,
            None => Some(now()),
        },
        stop: None,
        details: json!({}),
    }
}

impl TimingLogEvent {
    pub fn detail(&mut self, name: &str, value: &str) {
        self.details[name] = json!(value);
    }

    pub fn detail_json(&mut self, name: &str, value: &Value) {
        self.details[name] = value.clone();
    }

    pub fn finish(&mut self, when: Option<f64>) {
        self.stop = match when {
            Some(_) => when,
            None => Some(now()),
        };
    }
}

#[derive(Serialize)]
pub struct Timing {
    pub events: Vec<TimingLogEvent>,
}

impl Timing {
    pub fn new() -> Self {
        Timing { events: Vec::new() }
    }

    pub fn add(&mut self, e: TimingLogEvent) {
        self.events.push(e)
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use serde_json::{from_str, json, to_string, Value};

    fn ser(t: &Timing) -> Value {
        from_str(&to_string(t).unwrap()).unwrap()
    }

    #[test]
    fn test_build() {
        let mut t = Timing::new();
        assert_eq!(ser(&t), json!({"events": []}));

        let e1 = new_timelog("one", Some(1.0));
        t.add(e1);
        assert_eq!(
            ser(&t),
            json!({"events": [
                {"name": "one", "start": 1.0, "stop": null, "details": {}},
            ]})
        );

        let mut e2 = new_timelog("two", Some(2.0));
        e2.finish(Some(3.0));
        t.add(e2);
        assert_eq!(
            ser(&t),
            json!({"events": [
                {"name": "one", "start": 1.0, "stop": null, "details": {}},
                {"name": "two", "start": 2.0, "stop": 3.0, "details": {}},
            ]})
        );

        let mut e3 = new_timelog("three", Some(4.0));
        e3.detail("key1", "value1");
        e3.finish(Some(5.0));
        t.add(e3);
        assert_eq!(
            ser(&t),
            json!({"events": [
                {"name": "one", "start": 1.0, "stop": null, "details": {}},
                {"name": "two", "start": 2.0, "stop": 3.0, "details": {}},
                {"name": "three", "start": 4.0, "stop": 5.0,
                 "details": { "key1": "value1" }, },
            ]})
        );
    }

    #[test]
    fn test_json() {
        let mut t = Timing::new();
        let mut e = new_timelog("one", Some(1.0));
        e.detail_json("m", &json![{"foo": [1,2]}]);
        t.add(e);
        assert_eq!(
            ser(&t),
            json!({
                "events": [
                    {"name": "one", "start": 1.0, "stop": null,
                     "details": {"m": {"foo": [1, 2]},
                     },
                    },
                ]
            })
        );
    }

}
