pub enum BossEvent {
    RxWelcome,
    RxError,
    Error,
    Closed,
    GotCode,
    GotKey,
    Scared,
    Happy,
    GotVerifier,
    GotMessage,
}

use api::APIEvent;
use events::{BossResult, MachineEvent};
use rendezvous::RendezvousEvent; // TODO: only import what we use e.g. Stop

pub struct Boss {}

impl Boss {
    pub fn new() -> Boss {
        Boss {}
    }
    pub fn process_api_event(&mut self, event: APIEvent) -> Vec<BossResult> {
        match event {
            APIEvent::AllocateCode => vec![],
            APIEvent::SetCode(_code) => vec![],
            APIEvent::Close => vec![
                BossResult::Machine(MachineEvent::Rendezvous(RendezvousEvent::Stop)),
            ], // eventually signals GotClosed
            APIEvent::Send => vec![],
        }
    }

    pub fn execute(&mut self, _event: MachineEvent) -> Vec<BossResult> {
        vec![]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create() {
        let _b = Boss::new();
    }

    fn process_api() {
        let mut b = Boss::new();
        let actions = b.process_api_event(APIEvent::Close);
        assert_eq!(
            actions,
            vec![
                BossResult::Machine(MachineEvent::Rendezvous(RendezvousEvent::Stop)),
            ]
        );
    }
}
