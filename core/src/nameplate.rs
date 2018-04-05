use events::Event;
use events::Event::{N_Connected, N_Lost, N_NameplateDone, N_Release,
                    N_RxClaimed, N_RxReleased, N_SetNameplate};
use events::Event::{RC_TxClaim, RC_TxRelease};

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

    fn handle_connection(&mut self) -> Vec<Event> {
        let actions;
        let newstate = match self.state {
            State::S0A => {
                actions = vec![];
                State::S0B
            },
            State::S1A => {
                actions = vec![
                    RC_TxClaim
                ];
                State::S2B
            },
            State::S2A => {
                actions = vec![
                    RC_TxClaim
                ];
                State::S2B
            },
            State::S3A => {
                actions = vec![];
                State::S3B
            },
            State::S4A => {
                actions = vec![
                    RC_TxRelease
                ];
                State::S4B
            },
            State::S5A => {
                actions = vec![];
                State::S5B
            },
            _ => panic!() // TODO: handle S1A, S2A etc
        };
        self.state = newstate;
        actions
    }

    fn handle_lost(&mut self) -> Vec<Event> {
        let actions;
        let newstate = match self.state {
            State::S0B => {
                actions = vec![];
                State::S0A
            },
            State::S2B => {
                actions = vec![];
                State::S2A
            },
            State::S3B => {
                actions = vec![];
                State::S3A
            },
            State::S4B => {
                actions = vec![];
                State::S4A
            },
            State::S5B => {
                actions = vec![];
                State::S5A
            },
            _ => panic!() // got the Lost message while at an unexpected state
        };

        self.state = newstate;
        actions
    }
    
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            N_NameplateDone => vec![],
            N_Connected => self.handle_connection(),
            N_Lost => self.handle_lost(),
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
    use events::Event::{N_SetNameplate, N_Connected, N_Lost};

    #[test]
    fn create() {
        let mut n = super::Nameplate::new();

        // we are at S0A at init time. Now, we get N_Connected.
        // We enter S0B state and do not generate any events.
        let mut actions = n.process(N_Connected);
        assert_eq!(actions.len(), 0);

        // if we get the Lost event, we go back to S0A
        let mut actions = n.process(N_Lost);
        assert_eq!(actions.len(), 0);

        // now we get N_Connected again.
        let mut actions = n.process(N_Connected);
        assert_eq!(actions.len(), 0);

        // we are in State S0B, we get SetNameplate
        // we should set the nameplate and generate RC_TxClaim (and go to S2B)
        let mut actions = n.process(N_SetNameplate("42".to_string()));
        assert_eq!(actions.len(), 1);
    }
}
