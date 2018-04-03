use events::Event;
use api::Mood;
// we process these
use events::Event::{API_GotClosed, API_GotCode, API_GotMessage,
                    API_GotUnverifiedKey, API_GotVerifier, B_Closed, B_Error,
                    B_GotCode, B_GotKey, B_GotMessage, B_GotVerifier, B_Happy,
                    B_RxError, B_RxWelcome, B_Scared};
use events::Event::{API_AllocateCode, API_Close, API_Send, API_SetCode,
                    C_AllocateCode, C_InputCode, C_SetCode, S_Send};
// we emit these
use events::Event::{RC_Stop, T_Close};

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

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            API_AllocateCode => self.allocate_code(), // TODO: len, wordlist
            API_InputCode => self.input_code(),       // TODO: return Helper
            API_SetCode(code) => self.set_code(&code),
            B_GotCode(code) => self.got_code(&code),
            B_GotKey(key) => vec![API_GotUnverifiedKey(key)],
            B_Happy => self.happy(),
            B_GotVerifier(verifier) => vec![API_GotVerifier(verifier)],
            B_GotMessage(side, phase, plaintext) => {
                self.got_message(&side, &phase, plaintext)
            }
            API_Send(plaintext) => self.send(plaintext),
            API_Close => vec![RC_Stop], // eventually signals GotClosed
            B_Closed => self.closed(),
            B_Error | B_RxError | B_RxWelcome | B_Scared => vec![],
            _ => panic!(),
        }
    }

    fn allocate_code(&mut self) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => (vec![C_AllocateCode], Coding),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn set_code(&mut self, code: &str) -> Vec<Event> {
        // TODO: validate code, maybe signal KeyFormatError
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => (vec![C_SetCode(code.to_string())], Lonely),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn input_code(&mut self) -> Vec<Event> {
        // TODO: validate code, maybe signal KeyFormatError
        // TODO: return Helper somehow
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => (vec![C_InputCode], Coding),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn got_code(&mut self, code: &str) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Coding => (vec![API_GotCode(code.to_string())], Lonely),
            _ => panic!(), // TODO: signal AlreadyStartedCodeError
        };
        self.state = newstate;
        actions
    }

    fn happy(&mut self) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Lonely => (vec![], Happy),
            Closing => (vec![], Closing),
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
    ) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Closing => (vec![], Closing),
            Closed => (vec![], Closed),
            // TODO: find a way to combine these
            Empty => (vec![], Empty),
            Coding => (vec![], Coding),
            Lonely => (vec![], Lonely),
            Happy => {
                if phase == "version" {
                    // TODO deliver the "app_versions" key to API
                    (vec![], Happy)
                } else if phase == "\\d+" {
                    // TODO: match on regexp
                    (vec![API_GotMessage(plaintext)], Happy)
                } else {
                    // TODO: log and ignore, for future expansion
                    (vec![], Happy)
                }
            }
        };
        self.state = newstate;
        actions
    }

    fn send(&mut self, plaintext: Vec<u8>) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Closing => (vec![], Closing),
            Closed => (vec![], Closed),
            // TODO: find a way to combine these
            Empty => (vec![S_Send(plaintext)], Empty),
            Coding => (vec![S_Send(plaintext)], Coding),
            Lonely => (vec![S_Send(plaintext)], Lonely),
            Happy => (vec![S_Send(plaintext)], Happy),
        };
        self.state = newstate;
        actions
    }

    fn close(&mut self) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Empty => {
                self.mood = Mood::Lonely;
                (vec![T_Close(Mood::Lonely)], Closing)
            }
            Coding => {
                self.mood = Mood::Lonely;
                (vec![T_Close(Mood::Lonely)], Closing)
            }
            Lonely => {
                self.mood = Mood::Lonely;
                (vec![T_Close(Mood::Lonely)], Closing)
            }
            Happy => {
                self.mood = Mood::Happy;
                (vec![T_Close(Mood::Happy)], Closing)
            }
            Closing => (vec![], Closing),
            Closed => (vec![], Closed),
        };
        self.state = newstate;
        actions
    }

    fn closed(&mut self) -> Vec<Event> {
        use self::State::*;
        let (actions, newstate) = match self.state {
            Closing => (vec![API_GotClosed(self.mood)], Closing),
            _ => panic!(),
        };
        self.state = newstate;
        actions
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use events::Event::API_Close;

    #[test]
    fn create() {
        let _b = Boss::new();
    }

    fn process_api() {
        let mut b = Boss::new();
        let actions = b.process(API_Close);
        assert_eq!(actions, vec![RC_Stop]);
    }
}
