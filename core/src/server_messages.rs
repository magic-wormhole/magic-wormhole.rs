
#[derive(Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct Bind {
    #[serde(rename="type")]
    pub msg_type: String,
    pub appid: String,
    pub side: String,
}

pub fn bind(appid: &str, side: &str) -> Bind {
    Bind{msg_type: "bind".to_owned(),
         appid: appid.to_string(),
         side: side.to_string(),
    }
}
use serde_json;
use serde_json::Value;

pub fn bind_from_str(s: &str) -> Bind {
    serde_json::from_str(&s).unwrap()
}
