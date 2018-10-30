use super::events::{Events, Nameplate};
// we process these
use super::events::CodeEvent;
// we emit these
use super::events::AllocatorEvent::Allocate as A_Allocate;
use super::events::BossEvent::GotCode as B_GotCode;
use super::events::InputEvent::Start as I_Start;
use super::events::KeyEvent::GotCode as K_GotCode;
use super::events::NameplateEvent::SetNameplate as N_SetNameplate;

#[derive(Debug, PartialEq)]
enum State {
    Idle,
    InputtingNameplate,
    InputtingWords,
    Allocating,
    Known,
}

pub struct CodeMachine {
    state: State,
}

impl CodeMachine {
    pub fn new() -> CodeMachine {
        CodeMachine { state: State::Idle }
    }

    pub fn process(&mut self, event: CodeEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            Idle => self.in_idle(event),
            InputtingNameplate => self.in_inputting_nameplate(event),
            InputtingWords => self.in_inputting_words(event),
            Allocating => self.in_allocating(event),
            Known => self.in_known(&event),
        };

        if let Some(s) = newstate {
            self.state = s;
        }

        actions
    }

    fn in_idle(&mut self, event: CodeEvent) -> (Option<State>, Events) {
        use super::events::CodeEvent::*;
        match event {
            AllocateCode(wordlist) => {
                (Some(State::Allocating), events![A_Allocate(wordlist)])
            }
            InputCode => (Some(State::InputtingNameplate), events![I_Start]), // TODO: return Input object
            SetCode(code) => {
                // TODO: try!(validate_code(code))
                let code_string = code.to_string();
                let nc: Vec<&str> = code_string.splitn(2, '-').collect();
                let nameplate = Nameplate(nc[0].to_string());
                (
                    Some(State::Known),
                    events![
                        N_SetNameplate(nameplate.clone()),
                        B_GotCode(code.clone()),
                        K_GotCode(code.clone())
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
        use super::events::CodeEvent::*;
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
        use super::events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(..) => panic!(),
            GotNameplate(..) => panic!(),
            FinishedInput(code) => (
                Some(State::Known),
                events![B_GotCode(code.clone()), K_GotCode(code.clone())],
            ),
        }
    }

    fn in_allocating(&mut self, event: CodeEvent) -> (Option<State>, Events) {
        use super::events::CodeEvent::*;
        match event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(nameplate, code) => {
                // TODO: assert code.startswith(nameplate+"-")
                (
                    Some(State::Known),
                    events![
                        N_SetNameplate(nameplate.clone()),
                        B_GotCode(code.clone()),
                        K_GotCode(code.clone())
                    ],
                )
            }
            GotNameplate(..) => panic!(),
            FinishedInput(..) => panic!(),
        }
    }

    fn in_known(&mut self, event: &CodeEvent) -> (Option<State>, Events) {
        use super::events::CodeEvent::*;
        match *event {
            AllocateCode(..) => panic!(),
            InputCode => panic!(),
            SetCode(..) => panic!(),
            Allocated(..) => panic!(),
            GotNameplate(..) => panic!(),
            FinishedInput(..) => panic!(),
        }
    }
}
