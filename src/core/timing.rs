//use std::collections::HashMap;
use serde_derive::Serialize;
use std::time::SystemTime;

#[derive(Debug, PartialEq, Serialize)]
pub struct TimingLogEvent {
    name: String,
    start: Option<f64>,
    stop: Option<f64>,
    details: Vec<(String, String)>,
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

pub fn new_timelog(
    name: &str,
    when: Option<f64>,
    detail: Option<(&str, &str)>,
) -> TimingLogEvent {
    let start = match when {
        Some(_) => when,
        None => Some(now()),
    };
    let mut e = TimingLogEvent {
        name: name.to_string(),
        start,
        stop: None,
        details: Vec::new(),
    };
    if let Some((name, value)) = detail {
        e.details.push((name.to_string(), value.to_string()));
    }
    e
}

impl TimingLogEvent {
    pub fn detail(&mut self, name: &str, value: &str) {
        self.details.push((name.to_string(), value.to_string()));
    }

    pub fn finish(&mut self, when: Option<f64>, detail: Option<(&str, &str)>) {
        if let Some((name, value)) = detail {
            self.details.push((name.to_string(), value.to_string()));
        }
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

        let e1 = new_timelog("one", Some(2.0), None);
        t.add(e1);
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            ]})
        );

        let e2 = new_timelog("two", Some(3.0), Some(("key1", "value1")));
        t.add(e2);
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            {"name": "two", "start": 3.0, "stop": null,
             "details": [ ["key1", "value1"] ]},
            ]})
        );

        let mut e3 = new_timelog("three", Some(4.0), Some(("key2", "value2")));
        e3.finish(Some(5.0), None);
        t.add(e3);
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            {"name": "two", "start": 3.0, "stop": null,
             "details": [ ["key1", "value1"] ]},
            {"name": "three", "start": 4.0, "stop": 5.0,
             "details": [ ["key2", "value2"] ]},
            ]})
        );

        let mut e4 = new_timelog("four", Some(6.0), None);
        e4.finish(Some(7.0), Some(("key3", "value3")));
        t.add(e4);
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            {"name": "two", "start": 3.0, "stop": null,
             "details": [ ["key1", "value1"] ]},
            {"name": "three", "start": 4.0, "stop": 5.0,
             "details": [ ["key2", "value2"] ]},
            {"name": "four", "start": 6.0, "stop": 7.0,
             "details": [ ["key3", "value3"] ]},
            ]})
        );
    }

}
