use crate::{core::APPID_RAW, AppID};
use std::borrow::Cow;

pub const APP_CONFIG: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(Cow::Borrowed(APPID_RAW)),
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion::new(Some(FileTransferV2Mode::Send)),
    with_dilation: false,
};

#[derive(Clone, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(rename = "transfer")]
pub enum FileTransferV2Mode {
    Send,
    Receive,
    Connect,
}

#[derive(Clone, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct DilatedTransfer {
    mode: FileTransferV2Mode,
}

#[derive(Clone, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppVersion {
    // #[serde(default)]
    // abilities: Cow<'static, [Cow<'static, str>]>,
    // #[serde(default)]
    // transfer_v2: Option<AppVersionTransferV2Hint>,

    // XXX: we don't want to send "can-dilate" key for non-dilated
    // wormhole, would making this an Option help? i.e. when the value
    // is a None, we don't serialize that into the json and do it only
    // when it is a "Some" value?
    // overall versions payload is of the form:
    // b'{"can-dilate": ["1"], "dilation-abilities": [{"type": "direct-tcp-v1"}, {"type": "relay-v1"}], "app_versions": {"transfer": {"mode": "send", "features": {}}}}'

    //can_dilate: Option<[Cow<'static, str>; 1]>,
    //dilation_abilities: Cow<'static, [Ability; 2]>,
    #[serde(rename = "transfer")]
    app_versions: Option<DilatedTransfer>,
}

impl AppVersion {
    pub const fn new(mode: Option<FileTransferV2Mode>) -> Self {
        // let can_dilate: Option<[Cow<'static, str>; 1]> = if enable_dilation {
        //     Some([std::borrow::Cow::Borrowed("1")])
        // } else {
        //     None
        // };

        let option = match mode {
            Some(mode) => Some(DilatedTransfer { mode }),
            None => None,
        };

        Self {
            // abilities: Cow::Borrowed([Cow::Borrowed("transfer-v1"), Cow::Borrowed("transfer-v2")]),
            // transfer_v2: Some(AppVersionTransferV2Hint::new())
            // can_dilate: can_dilate,
            // dilation_abilities: std::borrow::Cow::Borrowed(&[
            //     Ability{ ty: std::borrow::Cow::Borrowed("direct-tcp-v1") },
            //     Ability{ ty: std::borrow::Cow::Borrowed("relay-v1") },
            // ]),
            app_versions: option,
        }
    }
}

impl Default for AppVersion {
    fn default() -> Self {
        Self::new(Some(FileTransferV2Mode::Send))
    }
}
