use super::api::Mood;
use super::events::{Code, Events, Nameplate, Phase};
use std::str::FromStr;
use std::sync::Arc;
use super::wordlist::default_wordlist;

use regex::Regex;
use serde_json;

// we process these
use super::api::APIEvent;
use super::events::BossEvent;
// we emit these
use super::api::APIAction;
use super::events::CodeEvent::{
    AllocateCode as C_AllocateCode, InputCode as C_InputCode,
    SetCode as C_SetCode,
};
use super::events::InputEvent::{
    ChooseNameplate as I_ChooseNameplate, ChooseWords as I_ChooseWords,
    RefreshNameplates as I_RefreshNameplates,
};
use super::events::RendezvousEvent::Start as RC_Start;
use super::events::SendEvent::Send as S_Send;
use super::events::TerminatorEvent::Close as T_Close;

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Unstarted(u32),
    Empty(u32),
    Coding(u32),
    Lonely(u32),
    Happy(u32),
    Closing,
    Closed,
}

pub struct BossMachine {
    state: State,
    mood: Mood,
}

impl State {
    pub fn increment_phase(self) -> Self {
        use self::State::*;
        match self {
            Unstarted(i) => Unstarted(i + 1),
            Empty(i) => Empty(i + 1),
            Coding(i) => Coding(i + 1),
            Lonely(i) => Lonely(i + 1),
            Happy(i) => Happy(i + 1),
            Closing => Closing,
            Closed => Closed,
        }
    }
}

impl BossMachine {
    pub fn new() -> BossMachine {
        BossMachine {
            state: State::Unstarted(0),
            mood: Mood::Lonely,
        }
    }

    pub fn process_api(&mut self, event: APIEvent) -> Events {
        use super::api::APIEvent::*;

        match event {
            Start => self.start(),
            AllocateCode(num_words) => self.allocate_code(num_words), // TODO: wordlist
            InputCode => self.input_code(), // TODO: return Helper
            InputHelperRefreshNameplates => {
                self.input_helper_refresh_nameplates()
            }
            InputHelperChooseNameplate(nameplate) => {
                self.input_helper_choose_nameplate(&nameplate)
            }
            InputHelperChooseWords(words) => {
                self.input_helper_choose_words(&words)
            }
            SetCode(code) => self.set_code(&code),
            Close => self.close(),
            Send(plaintext) => self.send(plaintext),
        }
    }

    pub fn process(&mut self, event: BossEvent) -> Events {
        use super::events::BossEvent::*;
        match event {
            GotCode(code) => self.got_code(&code),
            GotKey(key) => events![APIAction::GotUnverifiedKey(key.clone())],
            Happy => self.happy(),
            GotVerifier(verifier) => events![APIAction::GotVerifier(verifier)],
            GotMessage(phase, plaintext) => self.got_message(&phase, plaintext),
            Closed => self.closed(),
            Error | RxError | Scared => events![], // TODO
            RxWelcome(ref v) => events![APIAction::GotWelcome(v.clone())],
        }
    }

    fn start(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(i) => (events![RC_Start], Empty(i)),
            _ => panic!("only start once"),
        };
        self.state = newstate;
        actions
    }

    fn allocate_code(&mut self, num_words: usize) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Empty(i) => {
                // TODO: provide choice of wordlists
                let wordlist = Arc::new(default_wordlist(num_words));
                (events![C_AllocateCode(wordlist)], Coding(i))
            }
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn set_code(&mut self, code: &Code) -> Events {
        // TODO: validate code, maybe signal KeyFormatError
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            // we move to Coding instead of directly to Lonely because
            // Code::SetCode will signal us with Boss:GotCode in just a
            // moment, and by not special-casing set_code we get to use the
            // same flow for allocate_code and input_code
            Empty(i) => (events![C_SetCode(code.clone())], Coding(i)),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn input_code(&mut self) -> Events {
        // TODO: validate code, maybe signal KeyFormatError
        // TODO: return Helper somehow
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Empty(i) => (events![C_InputCode], Coding(i)),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn input_helper_refresh_nameplates(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Coding(i) => (events![I_RefreshNameplates], Coding(i)),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn input_helper_choose_nameplate(&mut self, nameplate: &str) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Coding(i) => (
                events![I_ChooseNameplate(Nameplate(nameplate.to_string()))],
                Coding(i),
            ),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn input_helper_choose_words(&mut self, word: &str) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Coding(i) => (events![I_ChooseWords(word.to_string())], Coding(i)),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn got_code(&mut self, code: &Code) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Coding(i) => (events![APIAction::GotCode(code.clone())], Lonely(i)),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn happy(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Lonely(i) => (events![], Happy(i)),
            Closing => (events![], Closing),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn got_message(&mut self, phase: &Phase, plaintext: Vec<u8>) -> Events {
        use self::State::*;
        let phase_pattern = Regex::from_str("\\d+").unwrap();
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Closing | Closed | Empty(..) | Coding(..) | Lonely(..) => {
                (events![], self.state)
            }
            Happy(i) => {
                if phase.to_string() == "version" {
                    // TODO handle error conditions
                    use serde_json::Value;
                    let version_str = String::from_utf8(plaintext).unwrap();
                    let v: Value = serde_json::from_str(&version_str).unwrap();
                    let app_versions = match v.get("app_versions") {
                        Some(versions) => versions.clone(),
                        None => json!({}),
                    };

                    (events![APIAction::GotVersions(app_versions)], Happy(i))
                } else if phase_pattern.is_match(phase) {
                    (events![APIAction::GotMessage(plaintext)], Happy(i))
                } else {
                    // TODO: log and ignore, for future expansion
                    (events![], Happy(i))
                }
            }
        };
        self.state = newstate;
        actions
    }

    fn send(&mut self, plaintext: Vec<u8>) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Closing | Closed => (events![], self.state),
            Empty(i) | Coding(i) | Lonely(i) | Happy(i) => (
                events![S_Send(Phase(format!("{}", i)), plaintext)],
                self.state.increment_phase(),
            ),
        };
        self.state = newstate;
        actions
    }

    fn close(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Empty(..) | Coding(..) | Lonely(..) => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Happy(..) => {
                self.mood = Mood::Happy;
                (events![T_Close(Mood::Happy)], Closing)
            }
            Closing => (events![], Closing),
            Closed => (events![], Closed),
        };
        self.state = newstate;
        actions
    }

    fn closed(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Unstarted(_) => panic!("w.start() must be called first"),
            Closing => (events![APIAction::GotClosed(self.mood)], Closed),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::api::{APIEvent, Mood};
    use core::events::{Key, RendezvousEvent, TerminatorEvent};

    #[test]
    fn create() {
        let _b = BossMachine::new();
    }

    #[test]
    fn process_api() {
        let mut b = BossMachine::new();
        let actions = b.process_api(APIEvent::Start);
        assert_eq!(actions, events![RendezvousEvent::Start]);

        let actions = b.process_api(APIEvent::Close);
        assert_eq!(actions, events![TerminatorEvent::Close(Mood::Lonely)]);
    }

    #[test]
    fn versions() {
        let mut b = BossMachine::new();
        use self::BossEvent::*;
        b.process_api(APIEvent::Start); // -> Started
        b.process_api(APIEvent::SetCode(Code("4-foo".to_string()))); // -> Coding
        b.process(GotCode(Code("4-foo".to_string()))); // -> Lonely
        b.process(GotKey(Key(b"".to_vec()))); // not actually necessary
        b.process(Happy);
        let v = json!({"for_wormhole": 123,
                       "app_versions": {
                           "hello_app": 456,
                       }}).to_string();
        let actions = b.process(GotMessage(
            Phase("version".to_string()),
            v.as_bytes().to_vec(),
        ));
        assert_eq!(
            actions,
            events![APIAction::GotVersions(json!({"hello_app": 456})),]
        );
    }

}
