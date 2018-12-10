use super::api::Mood;
use super::events::Phase;

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

use super::{Action, IOAction, IOEvent, TimerHandle, WSHandle, WormholeCore};
use crate::core::server_messages::{deserialize_outbound, OutboundMessage};

#[test]
fn create() {
    let url: &str = "url";
    let mut w = WormholeCore::new("appid", url);

    let wsh = WSHandle::new(1);
    let th = TimerHandle::new(2);
    let mut _got_side: &str;

    let ios = w.start();
    assert_eq!(ios.len(), 1);
    assert_eq!(
        ios,
        vec![Action::IO(IOAction::WebSocketOpen(wsh, url.to_string()))]
    );

    let actions = w.do_io(IOEvent::WebSocketConnectionMade(wsh));
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::IO(IOAction::WebSocketSendMessage(handle, m)) => {
            assert_eq!(handle, &wsh);
            if let OutboundMessage::Bind { appid, side } =
                deserialize_outbound(&m)
            {
                assert_eq!(appid, "appid");
                _got_side = &side; // random
            } else {
                panic!();
            }
        }
        _ => panic!(),
    }

    let actions = w.do_io(IOEvent::WebSocketConnectionLost(wsh));
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::IO(IOAction::StartTimer(handle, delay)) => {
            assert_eq!(handle, &th);
            assert_eq!(delay, &5.0);
        }
        _ => panic!(),
    }

    let actions = w.do_io(IOEvent::TimerExpired(th));
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions,
        vec![Action::IO(IOAction::WebSocketOpen(
            WSHandle::new(2),
            url.to_string()
        ))]
    );
}
