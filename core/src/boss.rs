use events::{Event, Events, Wordlist};
use api::Mood;
// we process these
use events::BossEvent;
use api::APIEvent;
// we emit these
use api::APIAction;
use events::CodeEvent::{AllocateCode as C_AllocateCode,
                        InputCode as C_InputCode, SetCode as C_SetCode};
use events::RendezvousEvent::Stop as RC_Stop;
use events::SendEvent::Send as S_Send;
use events::TerminatorEvent::Close as T_Close;

#[derive(Debug, PartialEq)]
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
            GotMessage(side, phase, plaintext) => {
                self.got_message(&side, &phase, plaintext)
            }
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
                (events![C_AllocateCode(length, wordlist)], Coding(i))
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
            Coding(i) => {
                (events![APIAction::GotCode(code.to_string())], Lonely(i))
            }
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

    fn got_message(
        &mut self,
        side: &str,
        phase: &str,
        plaintext: Vec<u8>,
    ) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Closing => (events![], Closing),
            Closed => (events![], Closed),
            // TODO: find a way to combine these
            Empty(i) => (events![], Empty(i)),
            Coding(i) => (events![], Coding(i)),
            Lonely(i) => (events![], Lonely(i)),
            Happy(i) => {
                if phase == "version" {
                    // TODO deliver the "app_versions" key to API
                    (events![], Happy(i))
                } else if phase == "\\d+" {
                    // TODO: match on regexp
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
            Closing => (events![], Closing),
            Closed => (events![], Closed),
            // TODO: find a way to combine these
            Empty(i) => {
                (events![S_Send(format!("{}", i), plaintext)], Empty(i + 1))
            }
            Coding(i) => {
                (events![S_Send(format!("{}", i), plaintext)], Coding(i + 1))
            }
            Lonely(i) => {
                (events![S_Send(format!("{}", i), plaintext)], Lonely(i + 1))
            }
            Happy(i) => {
                (events![S_Send(format!("{}", i), plaintext)], Happy(i + 1))
            }
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
            Closing => (events![APIAction::GotClosed(self.mood)], Closing),
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

    fn process_api() {
        let mut b = Boss::new();
        let actions = b.process_api(APIEvent::Close);
        assert_eq!(actions, events![RendezvousEvent::Stop]);
    }
}
