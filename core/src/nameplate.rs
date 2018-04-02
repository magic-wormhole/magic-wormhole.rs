use events::{Result};

#[derive(Debug, PartialEq)]
pub enum NameplateEvent {
    NameplateDone,
    Connected,
    Lost,
    RxClaimed,
    RxReleased,
    SetNameplate,
    Release,
}

pub struct Nameplate {
}

pub fn new() -> Nameplate {
    Nameplate { }
}

impl Nameplate {
    pub fn execute(&mut self, event: NameplateEvent) -> Vec<Result> {
        match event {
            NameplateEvent::NameplateDone => vec![],
            NameplateEvent::Connected => vec![],
            NameplateEvent::Lost => vec![],
            NameplateEvent::RxClaimed => vec![],
            NameplateEvent::RxReleased => vec![],
            NameplateEvent::SetNameplate => vec![],
            NameplateEvent::Release => vec![],
        }
    }
}
