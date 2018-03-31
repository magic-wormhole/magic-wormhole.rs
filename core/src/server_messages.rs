use serde_json;
use serde_json::{Value, Number};

#[derive(Serialize, Deserialize)]
struct Nameplate {
    id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
#[serde(tag="type")]
pub enum Message {
    Bind { appid: String, side: String },
    List,
    Nameplates { nameplates: Vec<Nameplate> },
    Allocate,
    Allocated { nameplate: String },
    Claim { nameplate: String },
    Claimed { mailbox: String },
    Release { nameplate: String }, // TODO: nominally optional
    Released { },
    Open { mailbox: String },
    Add { phase: String, body: String },
    Message { side: String, phase: String, body: String, id: String },
    Close { mailbox: String, mood: String },
    Closed,
    Ack,
    Ping { ping: Number }, // actually only integer
    Pong { pong: Number },
    //Error { error: String, orig: Message },
}

pub fn bind(appid: &str, side: &str) -> Message {
    Message::Bind{appid: appid.to_string(), side: side.to_string() }
}

pub fn deserialize(s: &str) -> Message {
    serde_json::from_str(&s).unwrap()
}
