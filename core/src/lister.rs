use events::Event;
use events::Event::{L_Connected, L_Lost, L_Refresh, L_RxNameplates};

pub struct Lister {}

impl Lister {
    pub fn new() -> Lister {
        Lister {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            L_Connected => vec![],
            L_Lost => vec![],
            L_RxNameplates => vec![],
            L_Refresh => vec![],
            _ => panic!(),
        }
    }
}
