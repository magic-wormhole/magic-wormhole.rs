#![allow(warnings)]

//use std::collections::HashMap;
use serde_derive::Serialize;
use std::time::SystemTime;

#[derive(Serialize)]
struct Event {
    name: String,
    start: Option<f64>,
    stop: Option<f64>,
    details: Vec<(String, String)>,
}

fn now() -> f64 {
    let dur =
        SystemTime::duration_since(&SystemTime::now(), SystemTime::UNIX_EPOCH)
            .unwrap();
    //dur.as_float_secs()
    let secs = dur.as_secs() as f64;
    let nanos = (dur.subsec_nanos() as f64) * 0.000_000_001;
    secs + nanos
}

impl Event {
    fn detail(&mut self, name: &str, value: &str) {
        self.details.push((name.to_string(), value.to_string()));
    }

    fn finish(&mut self, when: Option<f64>, detail: Option<(&str, &str)>) {
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
struct Timing {
    pub events: Vec<Event>,
}

impl Timing {
    pub fn new() -> Self {
        Timing { events: Vec::new() }
    }

    pub fn add(
        &mut self,
        name: &str,
        when: Option<f64>,
        detail: Option<(&str, &str)>,
    ) {
        let start = match when {
            Some(_) => when,
            None => Some(now()),
        };
        let mut e = Event {
            name: name.to_string(),
            start,
            stop: None,
            details: Vec::new(),
        };
        if let Some((name, value)) = detail {
            e.details.push((name.to_string(), value.to_string()));
        }
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

        t.add("one", Some(2.0), None);
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            ]})
        );

        t.add("two", Some(3.0), Some(("key1", "value1")));
        assert_eq!(
            ser(&t),
            json!({"events": [
            {"name": "one", "start": 2.0, "stop": null, "details": []},
            {"name": "two", "start": 3.0, "stop": null,
             "details": [ ["key1", "value1"] ]},
            ]})
        );
    }

}
