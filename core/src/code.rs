use events::Events;
// we process these
use events::CodeEvent;
// we emit these
use events::AllocatorEvent::Allocate as A_Allocate;
use events::BossEvent::GotCode as B_GotCode;
use events::InputEvent::Start as I_Start;
use events::KeyEvent::GotCode as K_GotCode;
use events::NameplateEvent::SetNameplate as N_SetNameplate;

#[derive(Debug, PartialEq)]
enum State {
    Idle,
    InputtingNameplate,
    InputtingWords,
    Allocating,
    Known,
}

pub struct Code {
    state: State,
}

impl Code {
    pub fn new() -> Code {
        Code {
            state: State::Idle,
        }
    }

    pub fn process(&mut self, event: CodeEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            Idle => self.in_idle(event),
            InputtingNameplate => self.in_inputting_nameplate(event),
            InputtingWords => self.in_inputting_words(event),
            Allocating => self.in_allocating(event),
            Known => self.in_known(event),
        };
        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        actions
    }

    fn in_idle(&mut self, event: CodeEvent) -> (Option<State>, Events) {
        use events::CodeEvent::*;
        match event {
            AllocateCode(length, wordlist) => (
                Some(State::Allocating),
                events![A_Allocate(length, wordlist)],
            ),
            InputCode => (
                Some(State::InputtingNameplate),
                events![I_Start],
            ), // TODO: return Input object
            SetCode(code) => {
                // TODO: try!(validate_code(code))
                let nc: Vec<&str> = code.splitn(2, "-").collect();
                let nameplate = nc[0];
                (
                    Some(State::Known),
                    events![
                        N_SetNameplate(nameplate.to_string()),
                        B_GotCode(code.to_string()),
                        K_GotCode(code.to_string())
                    ],
                )
            }
            Allocated(..) => panic!(),
            GotNameplate(..) => panic!(),
            FinishedInput(..) => panic!(),
        }
    }

    fn in_inputting_nameplate(
        &mut self,
        event: CodeEvent,
    ) -> (Option<State>, Events) {
        use events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(..) => panic!(),
            GotNameplate(nameplate) => (
                Some(State::InputtingWords),
                events![N_SetNameplate(nameplate)],
            ),
            FinishedInput(..) => panic!(),
        }
    }

    fn in_inputting_words(
        &mut self,
        event: CodeEvent,
    ) -> (Option<State>, Events) {
        use events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(..) => panic!(),
            GotNameplate(..) => panic!(),
            FinishedInput(code) => (
                Some(State::Known),
                events![
                    B_GotCode(code.to_string()),
                    K_GotCode(code.to_string())
                ],
            ),
        }
    }

    fn in_allocating(&mut self, event: CodeEvent) -> (Option<State>, Events) {
        use events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(nameplate, code) => {
                // TODO: assert code.startswith(nameplate+"-")
                (
                    Some(State::Known),
                    events![
                        N_SetNameplate(nameplate.to_string()),
                        B_GotCode(code.to_string()),
                        K_GotCode(code.to_string())
                    ],
                )
            }
            GotNameplate(..) => panic!(),
            FinishedInput(..) => panic!(),
        }
    }

    fn in_known(&mut self, event: CodeEvent) -> (Option<State>, Events) {
        use events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(..) => panic!(),
            GotNameplate(..) => panic!(),
            FinishedInput(..) => panic!(),
        }
    }
}
