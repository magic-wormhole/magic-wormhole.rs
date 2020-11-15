use super::api::Mood;
use super::events::{Events, Phase};
use super::wordlist::default_wordlist;
use serde_json::json;
use std::sync::Arc;

// we process these
use super::api::APIEvent;
use super::events::BossEvent;
// we emit these
use super::api::APIAction;
use super::events::CodeEvent::{AllocateCode as C_AllocateCode, SetCode as C_SetCode};
use super::events::SendEvent::Send as S_Send;
use super::events::TerminatorEvent::Close as T_Close;

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Empty(u64),
    Coding(u64),
    Lonely(u64),
    Happy(u64),
    Closing(Mood),
    Closed(Mood),
}

pub struct BossMachine {
    state: Option<State>,
}

impl BossMachine {
    pub fn new() -> BossMachine {
        BossMachine {
            state: Some(State::Empty(0)),
        }
    }

    pub fn process_api(&mut self, event: APIEvent) -> Events {
        use super::api::APIEvent::*;
        use State::*;

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Empty(i) => match event {
                AllocateCode(num_words) => {
                    // TODO: provide choice of wordlists
                    let wordlist = Arc::new(default_wordlist(num_words));
                    actions.push(C_AllocateCode(wordlist));
                    Coding(i)
                },
                SetCode(code) => {
                    // TODO: validate code, maybe signal KeyFormatError
                    // We move to Coding instead of directly to Lonely
                    // because Code::SetCode will signal us with Boss:GotCode
                    // in just a moment, and by not special-casing set_code
                    // we get to use the same flow for allocate_code and
                    // input_code
                    actions.push(C_SetCode(code));
                    Coding(i)
                },
                Send(plaintext) => {
                    actions.push(S_Send(Phase(format!("{}", i)), plaintext));
                    Empty(i + 1)
                },
                Close => {
                    actions.push(T_Close(Mood::Lonely));
                    Closing(Mood::Lonely)
                },
            },
            Coding(i) => match event {
                // TODO: allocate/input/set-code: signal AlreadyStartedCodeError
                Send(plaintext) => {
                    actions.push(S_Send(Phase(format!("{}", i)), plaintext));
                    Coding(i + 1)
                },
                Close => {
                    actions.push(T_Close(Mood::Lonely));
                    Closing(Mood::Lonely)
                },
                _ => panic!(),
            },
            Lonely(i) => match event {
                Send(plaintext) => {
                    actions.push(S_Send(Phase(format!("{}", i)), plaintext));
                    Lonely(i + 1)
                },
                Close => {
                    actions.push(T_Close(Mood::Lonely));
                    Closing(Mood::Lonely)
                },
                _ => panic!(),
            },
            Happy(i) => match event {
                Send(plaintext) => {
                    actions.push(S_Send(Phase(format!("{}", i)), plaintext));
                    Happy(i + 1)
                },
                Close => {
                    actions.push(T_Close(Mood::Happy));
                    Closing(Mood::Happy)
                },
                _ => panic!(),
            },
            Closing(_) | Closed(_) => panic!("No API calls after close"),
        });
        actions
    }

    pub fn process(&mut self, event: BossEvent) -> Events {
        use super::events::BossEvent::*;
        use State::*;

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Empty(_) => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                _ => panic!(),
            },
            Coding(i) => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                GotCode(code) => {
                    actions.push(APIAction::GotCode(code));
                    Lonely(i)
                },
                _ => panic!(),
            },
            Lonely(i) => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                GotKey(key) => {
                    actions.push(APIAction::GotUnverifiedKey(key));
                    old_state
                },
                BossEvent::Happy => State::Happy(i),
                _ => panic!(),
            },
            State::Happy(_) => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                GotVerifier(verifier) => {
                    actions.push(APIAction::GotVerifier(verifier));
                    old_state
                },
                GotMessage(phase, plaintext) => {
                    if phase.is_version() {
                        // TODO handle error conditions
                        use serde_json::Value;
                        let version_str = String::from_utf8(plaintext).unwrap();
                        let v: Value = serde_json::from_str(&version_str).unwrap();
                        let app_versions = match v.get("app_versions") {
                            Some(versions) => versions.clone(),
                            None => json!({}),
                        };
                        actions.push(APIAction::GotVersions(app_versions));
                    } else if phase.to_num().is_some() {
                        actions.push(APIAction::GotMessage(plaintext));
                    } else {
                        // TODO: log and ignore, for future expansion
                        todo!("log and ignore, for future expansion");
                    }
                    old_state
                },
                // Scared: TODO
                _ => panic!(),
            },
            Closing(mood) => match event {
                RxWelcome(..) => old_state,
                BossEvent::Happy => old_state,
                BossEvent::Closed => {
                    actions.push(APIAction::GotClosed(mood));
                    State::Closed(mood)
                },
                _ => panic!(),
            },
            State::Closed(_) => panic!("No events after closed"),
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::api::{APIEvent, Mood};
    use crate::core::events::{Code, Key, TerminatorEvent};

    #[test]
    fn create() {
        let _b = BossMachine::new();
    }

    #[test]
    fn process_api() {
        let mut b = BossMachine::new();

        let actions = b.process_api(APIEvent::Close);
        assert_eq!(actions, events![TerminatorEvent::Close(Mood::Lonely)]);
    }

    #[test]
    fn versions() {
        let mut b = BossMachine::new();
        use self::BossEvent::*;
        b.process_api(APIEvent::SetCode(Code(String::from("4-foo")))); // -> Coding
        b.process(GotCode(Code(String::from("4-foo")))); // -> Lonely
        b.process(GotKey(Key(b"".to_vec()))); // not actually necessary
        b.process(Happy);
        let v = json!({"for_wormhole": 123,
        "app_versions": {
            "hello_app": 456,
        }})
        .to_string();
        let actions = b.process(GotMessage(
            Phase(String::from("version")),
            v.as_bytes().to_vec(),
        ));
        assert_eq!(
            actions,
            events![APIAction::GotVersions(json!({"hello_app": 456})),]
        );
    }
}
