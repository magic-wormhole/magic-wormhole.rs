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
    state: Option<State>,
}

impl CodeMachine {
    pub fn new() -> CodeMachine {
        CodeMachine {
            state: Some(State::Idle),
        }
    }

    pub fn process(&mut self, event: CodeEvent) -> Events {
        use CodeEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Idle => match event {
                AllocateCode(wordlist) => {
                    actions.push(A_Allocate(wordlist));
                    Allocating
                }
                InputCode => {
                    actions.push(I_Start);
                    // TODO: return Input object
                    InputtingNameplate
                }
                SetCode(code) => {
                    // TODO: try!(validate_code(code))
                    let code_string = code.to_string();
                    let nc: Vec<&str> = code_string.splitn(2, '-').collect();
                    let nameplate = Nameplate::new(nc[0]);
                    actions.push(N_SetNameplate(nameplate.clone()));
                    actions.push(B_GotCode(code.clone()));
                    actions.push(K_GotCode(code.clone()));
                    Known
                }
                _ => panic!(),
            },

            InputtingNameplate => match event {
                GotNameplate(nameplate) => {
                    actions.push(N_SetNameplate(nameplate));
                    InputtingWords
                }
                _ => panic!(),
            },
            InputtingWords => match event {
                FinishedInput(code) => {
                    actions.push(B_GotCode(code.clone()));
                    actions.push(K_GotCode(code.clone()));
                    Known
                }
                _ => panic!(),
            },
            Allocating => match event {
                Allocated(nameplate, code) => {
                    // TODO: assert code.startswith(nameplate+"-")
                    actions.push(N_SetNameplate(nameplate.clone()));
                    // TODO: maybe tell Key before Boss?
                    actions.push(B_GotCode(code.clone()));
                    actions.push(K_GotCode(code.clone()));
                    Known
                }
                _ => panic!(),
            },
            Known => panic!(),
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::super::events::{
        BossEvent, Code, Event, KeyEvent, NameplateEvent,
    };
    use super::CodeEvent::*;
    use super::CodeMachine;

    fn assert_keyboss(mut e: Vec<Event>) {
        // the last step of all successful CodeMachine paths is to notify
        // both the Key and Boss about the new code
        match e.remove(0) {
            Event::Boss(BossEvent::GotCode(c2)) => {
                assert_eq!(c2.to_string(), "4-purple-sausages");
            }
            _ => panic!(),
        }
        match e.remove(0) {
            Event::Key(KeyEvent::GotCode(c2)) => {
                assert_eq!(c2.to_string(), "4-purple-sausages");
            }
            _ => panic!(),
        }
        assert_eq!(e.len(), 0);
    }

    #[test]
    fn test_set_code() {
        let mut c = CodeMachine::new();
        let code = Code(String::from("4-purple-sausages"));
        let mut e = c.process(SetCode(code)).events;

        match e.remove(0) {
            Event::Nameplate(NameplateEvent::SetNameplate(n)) => {
                assert_eq!(n.to_string(), "4");
            }
            _ => panic!(),
        }
        assert_keyboss(e);
    }

    #[test]
    fn test_allocate_code() {
        use super::super::events::{AllocatorEvent, Nameplate};
        use super::super::wordlist::default_wordlist;
        use std::sync::Arc;
        let mut c = CodeMachine::new();
        let w = Arc::new(default_wordlist(2));
        let mut e = c.process(AllocateCode(w.clone())).events;

        match e.remove(0) {
            Event::Allocator(AllocatorEvent::Allocate(w2)) => {
                assert_eq!(w2, w);
            }
            _ => panic!(),
        }
        assert_eq!(e.len(), 0);

        e = c
            .process(Allocated(
                Nameplate::new("4"),
                Code(String::from("4-purple-sausages")),
            ))
            .events;
        match e.remove(0) {
            Event::Nameplate(NameplateEvent::SetNameplate(n)) => {
                assert_eq!(n.to_string(), "4");
            }
            _ => panic!(),
        }
        assert_keyboss(e);
    }

    #[test]
    fn test_input_code() {
        use super::super::events::{InputEvent, Nameplate};
        let mut c = CodeMachine::new();
        let mut e = c.process(InputCode).events;

        match e.remove(0) {
            Event::Input(InputEvent::Start) => (),
            _ => panic!(),
        }
        assert_eq!(e.len(), 0);

        let n = Nameplate::new("4");
        e = c.process(GotNameplate(n)).events;
        match e.remove(0) {
            Event::Nameplate(NameplateEvent::SetNameplate(n)) => {
                assert_eq!(n.to_string(), "4");
            }
            _ => panic!(),
        }
        assert_eq!(e.len(), 0);

        let code = Code(String::from("4-purple-sausages"));
        e = c.process(FinishedInput(code)).events;
        assert_keyboss(e);
    }

}
