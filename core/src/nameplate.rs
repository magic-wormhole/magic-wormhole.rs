use events::Event;
use events::Event::{N_Connected, N_Lost, N_NameplateDone, N_Release,
                    N_RxClaimed, N_RxReleased, N_SetNameplate};

pub struct Nameplate {}

pub fn new() -> Nameplate {
    Nameplate {}
}

impl Nameplate {
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            N_NameplateDone => vec![],
            N_Connected => vec![],
            N_Lost => vec![],
            N_RxClaimed => vec![],
            N_RxReleased => vec![],
            N_SetNameplate => vec![],
            N_Release => vec![],
            _ => panic!(),
        }
    }
}
