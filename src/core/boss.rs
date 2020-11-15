use crate::core::events::KeyEvent;
use crate::core::events::NameplateEvent;
use crate::core::events::Nameplate;
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
use super::events::MailboxEvent::Close as M_Close;
use super::events::NameplateEvent::Close as N_Close;
use super::events::RendezvousEvent::Stop as RC_Stop;
use super::events::SendEvent::Send as S_Send;
use super::events::RendezvousEvent::TxAllocate as RC_TxAllocate;
use super::events::{Code, Wordlist};

enum State {
    Empty,
    Coding(Arc<Wordlist>),
    Lonely,
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
            state: Some(State::Empty),
        }
    }

    pub fn process_api(&mut self, event: APIEvent) -> Events {
        use super::api::APIEvent::*;
        use State::*;

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Empty => match event {
                AllocateCode(num_words) => {
                    // TODO: provide choice of wordlists
                    let wordlist = Arc::new(default_wordlist(num_words));

                    actions.push(RC_TxAllocate);

                    Coding(wordlist)
                },
                SetCode(code) => {
                    // TODO: validate code, maybe signal KeyFormatError
                    let code_string = code.to_string();
                    let nc: Vec<&str> = code_string.splitn(2, '-').collect();
                    let nameplate = Nameplate::new(nc[0]);
                    actions.push(NameplateEvent::SetNameplate(nameplate));
                    actions.push(KeyEvent::GotCode(code.clone()));
                    actions.push(APIAction::GotCode(code));

                    Lonely
                },
                Send(_) => unreachable!("Sending messages before after PAKE should be prohibited by outer API layers"),
                Close => {
                    actions.push(N_Close);
                    actions.push(M_Close(Mood::Lonely));
                    actions.push(RC_Stop);

                    Closing(Mood::Lonely)
                },
            },
            Coding(_) => match event {
                // TODO: allocate/input/set-code: signal AlreadyStartedCodeError
                Send(_) => unreachable!("Sending messages before after PAKE should be prohibited by outer API layers"),
                Close => {
                    actions.push(N_Close);
                    actions.push(M_Close(Mood::Lonely));
                    actions.push(RC_Stop);

                    Closing(Mood::Lonely)
                },
                _ => panic!(),
            },
            Lonely => match event {
                Send(_) => unreachable!("Sending messages before after PAKE should be prohibited by outer API layers"),
                Close => {
                    actions.push(N_Close);
                    actions.push(M_Close(Mood::Lonely));
                    actions.push(RC_Stop);

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
                    actions.push(N_Close);
                    actions.push(M_Close(Mood::Happy));
                    actions.push(RC_Stop);

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
            Empty => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                _ => panic!(),
            },
            Coding(wordlist) => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    Coding(wordlist)
                },
                Allocated(nameplate) => {
                    let words = wordlist.choose_words();
                    let code = Code(nameplate.to_string() + "-" + &words);

                    // TODO: assert code.startswith(nameplate+"-")
                    actions.push(NameplateEvent::SetNameplate(nameplate));
                    actions.push(KeyEvent::GotCode(code.clone()));
                    actions.push(APIAction::GotCode(code));

                    Lonely
                },
                _ => panic!(),
            },
            Lonely => match event {
                RxWelcome(v) => {
                    actions.push(APIAction::GotWelcome(v));
                    old_state
                },
                GotKey(key) => {
                    actions.push(APIAction::GotUnverifiedKey(key));
                    old_state
                },
                BossEvent::Happy => State::Happy(0),
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
    use crate::core::api::APIEvent;
    use crate::core::events::{Code, Key};

    #[test]
    fn create() {
        let _b = BossMachine::new();
    }

    #[test]
    fn versions() {
        let mut b = BossMachine::new();
        b.process_api(APIEvent::SetCode(Code(String::from("4-foo")))); // -> Coding
        b.process(BossEvent::GotCode(Code(String::from("4-foo")))); // -> Lonely
        b.process(BossEvent::GotKey(Key(b"".to_vec()))); // not actually necessary
        b.process(BossEvent::Happy);
        let v = json!({"for_wormhole": 123,
        "app_versions": {
            "hello_app": 456,
        }})
        .to_string();
        let actions = b.process(BossEvent::GotMessage(
            Phase(String::from("version")),
            v.as_bytes().to_vec(),
        ));
        assert_eq!(
            actions,
            events![APIAction::GotVersions(json!({"hello_app": 456})),]
        );
    }
}
