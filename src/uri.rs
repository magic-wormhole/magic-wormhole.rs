/// Custom magic wormhole URI scheme
///
/// At the moment, only `wormhole-transfer:` is specified as scheme
/// and therefore URLs can only be used for file transfer applications.
/// This, however, might change in the future.
use super::*;

#[derive(Debug, thiserror::Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParseError {
    #[error("Wrong URI scheme, must be 'wormhole-transfer' but was '{_0}'")]
    SchemeError(String),
    #[error("Wormhole URIs start with 'wormhole-transfer:${{code}}', they do not have a host")]
    HasHost,
    #[error("Code is missing or empty")]
    MissingCode,
    #[error("Unsupported scheme version {_0}")]
    UnsupportedVersion(String),
    #[error("Invalid 'role' parameter: '{_0}'")]
    InvalidRole(String),
    /// Some deserialization went wrong, we probably got some garbage
    #[error("String does not parse as URL")]
    UrlParseError(
        #[from]
        #[source]
        url::ParseError,
    ),
    #[error("Invalid UTF-8 encoding: {_0}")]
    Utf8Error(
        #[from]
        #[source]
        std::str::Utf8Error,
    ),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WormholeTransferUri {
    pub code: Code,
    /// If `Some`, a custom non-default rendezvous-server is being requested
    pub rendezvous_server: Option<url::Url>,
    /// By default, the "leader" (e.g. the file sender) generates the code (and thus the link),
    /// while the "follower" (receiver) parses the code. However, since not all devices can
    /// parse QR images equally well, this dynamic can be inversed.
    ///
    /// For example, when sending a file from a smart phone to a computer, one would initiate the
    /// transfer from the computer side (and thus set `is_leader` to `true`), because only the phone
    /// has a camera.
    pub is_leader: bool,
}

impl WormholeTransferUri {
    pub fn new(code: Code) -> Self {
        Self {
            code,
            rendezvous_server: None,
            is_leader: false,
        }
    }
}

impl TryFrom<&url::Url> for WormholeTransferUri {
    type Error = ParseError;

    fn try_from(url: &url::Url) -> Result<Self, ParseError> {
        use std::ops::Deref;

        match url.scheme() {
            "wormhole-transfer" => {},
            other => return Err(ParseError::SchemeError(other.into())),
        }
        if url.has_host() {
            return Err(ParseError::HasHost);
        }
        let queries = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        match queries.get("version").map(Deref::deref).unwrap_or("0") {
            version if version == "0" => {},
            unsupported => return Err(ParseError::UnsupportedVersion(unsupported.into())),
        }
        let rendezvous_server = queries
            .get("rendezvous")
            .map(Deref::deref)
            .map(url::Url::parse)
            .transpose()?;
        let is_leader = match queries.get("role").map(Deref::deref).unwrap_or("follower") {
            "leader" => true,
            "follower" => false,
            invalid => return Err(ParseError::InvalidRole(invalid.into())),
        };
        let code = Code(
            percent_encoding::percent_decode_str(url.path())
                .decode_utf8()?
                .into(),
        );
        // TODO move the code validation to somewhere else and also add more checks
        if code.is_empty() {
            return Err(ParseError::MissingCode);
        }

        Ok(WormholeTransferUri {
            code,
            rendezvous_server,
            is_leader,
        })
    }
}

impl TryFrom<url::Url> for WormholeTransferUri {
    type Error = ParseError;

    fn try_from(url: url::Url) -> Result<Self, ParseError> {
        (&url).try_into()
    }
}

impl std::str::FromStr for WormholeTransferUri {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        url::Url::parse(s)?.try_into()
    }
}

impl From<&WormholeTransferUri> for url::Url {
    fn from(val: &WormholeTransferUri) -> Self {
        let mut url = url::Url::parse("wormhole-transfer:").unwrap();
        url.set_path(&val.code);
        /* Only do this if there are any query parameteres at all, otherwise the URL will have an ugly trailing '?'. */
        if val.rendezvous_server.is_some() || val.is_leader {
            let mut query = url.query_pairs_mut();
            query.clear();
            if let Some(rendezvous_server) = val.rendezvous_server.as_ref() {
                query.append_pair("rendezvous", rendezvous_server.as_ref());
            }
            if val.is_leader {
                query.append_pair("role", "leader");
            }
        }
        url
    }
}

impl std::fmt::Display for WormholeTransferUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        url::Url::from(self).fmt(f)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn test_eq(parsed: WormholeTransferUri, string: &str) {
        assert_eq!(parsed.to_string(), string);
        assert_eq!(string.parse::<WormholeTransferUri>().unwrap(), parsed);
    }

    #[test]
    fn test_uri() {
        test_eq(
            WormholeTransferUri::new(Code("4-hurricane-equipment".to_owned())),
            "wormhole-transfer:4-hurricane-equipment",
        );

        test_eq(
            WormholeTransferUri::new(Code("8-🙈-🙉-🙊".to_owned())),
            "wormhole-transfer:8-%F0%9F%99%88-%F0%9F%99%89-%F0%9F%99%8A",
        );

        test_eq(
            WormholeTransferUri {
                code: Code("8-🙈-🙉-🙊".to_owned()),
                rendezvous_server: Some(url::Url::parse("ws://localhost:4000").unwrap()),
                is_leader: true,
            },
            "wormhole-transfer:8-%F0%9F%99%88-%F0%9F%99%89-%F0%9F%99%8A?rendezvous=ws%3A%2F%2Flocalhost%3A4000%2F&role=leader"
        );
    }

    #[test]
    fn test_uri_err() {
        assert_eq!(
            "wormhole-transfer:8-%F0%9F%99%88-%F0%9F%99%89-%F0%9F%99%8A?version=42&rendezvous=ws%3A%2F%2Flocalhost%3A4000%2F&role=leader".parse::<WormholeTransferUri>(),
            Err(ParseError::UnsupportedVersion("42".into()))
        );
        assert_eq!(
            "wormhole-transfer:?rendezvous=ws%3A%2F%2Flocalhost%3A4000%2F&role=leader"
                .parse::<WormholeTransferUri>(),
            Err(ParseError::MissingCode)
        );
    }
}
