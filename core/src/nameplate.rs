use events::Event;
use events::Event::{N_Connected, N_Lost, N_NameplateDone, N_Release,
                    N_RxClaimed, N_RxReleased, N_SetNameplate};
use events::Event::{RC_TxClaim};

#[derive(Debug, PartialEq)]
enum State {
    // S0
    S0A,
    S0B,
    // S1
    S1A,
    // S2
    S2A,
    S2B,
    // S3
    S3A,
    S3B,
    // S4
    S4A,
    S4B,
    // S5
    S5A,
    S5B,
    S5
}

pub struct Nameplate {
    nameplate: Option<u32>,
    state: State
}

impl Nameplate {
    pub fn new() -> Nameplate {
        Nameplate {
            nameplate: None,
            state: State::S0A
        }
    }

    fn validate_nameplate(&mut self, nameplate: &str) -> Option<u32> {
        nameplate.parse::<u32>().ok()
    }

    fn set_nameplate(&mut self, nameplate_input: String) -> Vec<Event> {
        let actions;
        let newstate = match self.state {
            State::S0A => {
                self.nameplate = self.validate_nameplate(&nameplate_input);
                actions = vec![];
                State::S1A
            },
            State::S0B => {
                // record nameplate and send claim message
                self.nameplate = self.validate_nameplate(&nameplate_input);
                // return RC_TxClaim event
                actions = vec![
                    RC_TxClaim
                ];
                State::S2B
            },
            _ => panic!()
        };

        self.state = newstate;
        actions
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            N_NameplateDone => vec![],
            N_Connected => vec![],
            N_Lost => vec![],
            N_RxClaimed => vec![],
            N_RxReleased => vec![],
            N_SetNameplate(nameplate) => self.set_nameplate(nameplate),
            N_Release => vec![],
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod test {
    use events::Event::{N_SetNameplate};

    #[test]
    fn create() {
        let mut n = super::Nameplate::new();

        let mut actions = n.process(N_SetNameplate("42".to_string()));
        assert_eq!(actions.len(), 0);
    }
}
