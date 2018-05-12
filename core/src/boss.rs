use api::Mood;
use events::{Event, Events, Wordlist};
// we process these
use api::APIEvent;
use events::BossEvent;
// we emit these
use api::APIAction;
use events::CodeEvent::{AllocateCode as C_AllocateCode,
                        InputCode as C_InputCode, SetCode as C_SetCode};
use events::InputEvent::ChooseWords as I_ChooseWords;
use events::RendezvousEvent::Stop as RC_Stop;
use events::SendEvent::Send as S_Send;
use events::TerminatorEvent::Close as T_Close;

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Empty(u32),
    Coding(u32),
    Lonely(u32),
    Happy(u32),
    Closing,
    Closed,
}

pub struct Boss {
    state: State,
    mood: Mood,
}

impl State {
    pub fn increment_phase(&self) -> Self {
        use self::State::*;
        match *self {
            Empty(i) => Empty(i + 1),
            Coding(i) => Coding(i + 1),
            Lonely(i) => Lonely(i + 1),
            Happy(i) => Happy(i + 1),
            Closing => Closing,
            Closed => Closed,
        }
    }
}
impl Boss {
    pub fn new() -> Boss {
        Boss {
            state: State::Empty(0),
            mood: Mood::Lonely,
        }
    }

    pub fn process_api(&mut self, event: APIEvent) -> Events {
        use api::APIEvent::*;
        match event {
            AllocateCode => self.allocate_code(), // TODO: len, wordlist
            InputCode => self.input_code(),       // TODO: return Helper
            SetCode(code) => self.set_code(&code),
            HelperChoseWord(word) => self.choose_word(&word),
            Close => events![RC_Stop], // eventually signals GotClosed
            Send(plaintext) => self.send(plaintext),
        }
    }

    pub fn process(&mut self, event: BossEvent) -> Events {
        use events::BossEvent::*;
        match event {
            GotCode(code) => self.got_code(&code),
            GotKey(key) => events![APIAction::GotUnverifiedKey(key)],
            Happy => self.happy(),
            GotVerifier(verifier) => events![APIAction::GotVerifier(verifier)],
            GotMessage(phase, plaintext) => self.got_message(&phase, plaintext),
            Closed => self.closed(),
            Error | RxError | RxWelcome | Scared => events![],
        }
    }

    fn allocate_code(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty(i) => {
                let length = 2; // TODO: configurable by AllocateCode
                let wordlist = Wordlist {}; // TODO: populate words
                (
                    events![C_AllocateCode(length, wordlist)],
                    Coding(i),
                )
            }
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn set_code(&mut self, code: &str) -> Events {
        // TODO: validate code, maybe signal KeyFormatError
        use self::State::*;
        let (actions, newstate) = match self.state {
            // we move to Coding instead of directly to Lonely because
            // Code::SetCode will signal us with Boss:GotCode in just a
            // moment, and by not special-casing set_code we get to use the
            // same flow for allocate_code and input_code
            Empty(i) => (events![C_SetCode(code.to_string())], Coding(i)),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn choose_word(&mut self, word: &str) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Coding(i) => (
                events![I_ChooseWords(word.to_string())],
                Coding(i),
            ),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn input_code(&mut self) -> Events {
        // TODO: validate code, maybe signal KeyFormatError
        // TODO: return Helper somehow
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty(i) => (events![C_InputCode], Coding(i)),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn got_code(&mut self, code: &str) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Coding(i) => (
                events![APIAction::GotCode(code.to_string())],
                Lonely(i),
            ),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn happy(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Lonely(i) => (events![], Happy(i)),
            Closing => (events![], Closing),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }

    fn got_message(&mut self, phase: &str, plaintext: Vec<u8>) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Closing | Closed | Empty(..) | Coding(..) | Lonely(..) => {
                (events![], self.state)
            }
            Happy(i) => {
                if phase == "version" {
                    // TODO deliver the "app_versions" key to API
                    (events![], Happy(i))
                } else if phase == "\\d+" {
                    // TODO: match on regexp
                    (
                        events![APIAction::GotMessage(plaintext)],
                        Happy(i),
                    )
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
            Closing | Closed => (events![], self.state),
            Empty(i) | Coding(i) | Lonely(i) | Happy(i) => (
                events![S_Send(format!("{}", i), plaintext)],
                self.state.increment_phase(),
            ),
        };
        self.state = newstate;
        actions
    }

    fn close(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty(i) => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Coding(i) => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Lonely(i) => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Happy(i) => {
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
            Closing => (
                events![APIAction::GotClosed(self.mood)],
                Closing,
            ),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use api::APIEvent;
    use events::RendezvousEvent;

    #[test]
    fn create() {
        let _b = Boss::new();
    }

    #[test]
    fn process_api() {
        let mut b = Boss::new();
        let actions = b.process_api(APIEvent::Close);
        assert_eq!(actions, events![RendezvousEvent::Stop]);
    }
}
