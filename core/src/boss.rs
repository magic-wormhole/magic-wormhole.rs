use events::{Event, Events};
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
    Empty,
    Coding,
    Lonely,
    Happy,
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
            state: State::Empty,
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
            Empty => (events![C_AllocateCode], Coding),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn set_code(&mut self, code: &str) -> Events {
        // TODO: validate code, maybe signal KeyFormatError
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => (events![C_SetCode(code.to_string())], Lonely),
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
            Empty => (events![C_InputCode], Coding),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn got_code(&mut self, code: &str) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Coding => (events![APIAction::GotCode(code.to_string())], Lonely),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn happy(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Lonely => (events![], Happy),
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
            Empty => (events![], Empty),
            Coding => (events![], Coding),
            Lonely => (events![], Lonely),
            Happy => {
                if phase == "version" {
                    // TODO deliver the "app_versions" key to API
                    (events![], Happy)
                } else if phase == "\\d+" {
                    // TODO: match on regexp
                    (events![APIAction::GotMessage(plaintext)], Happy)
                } else {
                    // TODO: log and ignore, for future expansion
                    (events![], Happy)
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
            Empty => (events![S_Send(plaintext)], Empty),
            Coding => (events![S_Send(plaintext)], Coding),
            Lonely => (events![S_Send(plaintext)], Lonely),
            Happy => (events![S_Send(plaintext)], Happy),
        };
        self.state = newstate;
        actions
    }

    fn close(&mut self) -> Events {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Coding => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Lonely => {
                self.mood = Mood::Lonely;
                (events![T_Close(Mood::Lonely)], Closing)
            }
            Happy => {
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
