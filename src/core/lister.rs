use super::events::Events;
// we process these
use super::events::ListerEvent;
// we emit these
use super::events::InputEvent::GotNameplates as I_GotNameplates;
use super::events::ListerEvent::*;
use super::events::RendezvousEvent::TxList as RC_TxList;

pub struct ListerMachine {
    state: Option<State>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    // All A's unconnected
    // All B's connected

    // not wanting list unconnected
    S0A,
    // Want list unconnected
    S1A,
    // not wanting list connected
    S0B,

    // want list connected
    S1B,
}

impl ListerMachine {
    pub fn new() -> ListerMachine {
        ListerMachine {
            state: Some(State::S0A),
        }
    }

    pub fn process(&mut self, event: ListerEvent) -> Events {
        use self::State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0A => match event {
                Connected => S0B,
                Refresh => S1A,
                _ => old_state,
            },
            S0B => match event {
                Refresh => {
                    actions.push(RC_TxList);
                    S1B
                }
                Lost => S0A,
                RxNameplates(nids) => {
                    // We didn't explicitly need the nameplates, but we got
                    // them anyway for some reason. Give them to Input so
                    // they'll be ready in case user wants completion.
                    actions.push(I_GotNameplates(nids.clone()));
                    old_state
                }
                Connected => old_state,
            },
            S1A => match event {
                Connected => {
                    actions.push(RC_TxList);
                    S1B
                }
                Refresh => old_state,
                Lost => old_state,
                RxNameplates(_) => {
                    panic!("not connected, shouldn't get nameplates")
                }
            },
            S1B => match event {
                Lost => S1A,
                Refresh => {
                    actions.push(RC_TxList);
                    old_state
                }
                RxNameplates(nids) => {
                    actions.push(I_GotNameplates(nids.clone()));
                    S0B
                }
                Connected => old_state,
            },
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::{ListerMachine, State};
    use crate::core::events::{
        InputEvent::GotNameplates, ListerEvent::*, Nameplate,
        RendezvousEvent::TxList,
    };

    #[test]
    fn test_lister() {
        let mut lister = ListerMachine::new();

        assert_eq!(lister.state, Some(State::S0A));

        assert_eq!(lister.process(Connected), events![]);
        assert_eq!(lister.state, Some(State::S0B));

        assert_eq!(lister.process(Lost), events![]);
        assert_eq!(lister.state, Some(State::S0A));

        lister.state = Some(State::S0B);
        let nameplates: Vec<Nameplate> =
            vec!["3"].into_iter().map(|s| Nameplate::new(s)).collect();
        assert_eq!(
            lister.process(RxNameplates(nameplates.clone())),
            events![GotNameplates(nameplates)]
        );
        assert_eq!(lister.state, Some(State::S0B));

        assert_eq!(lister.process(Refresh), events![TxList]);
        assert_eq!(lister.state, Some(State::S1B));

        assert_eq!(lister.process(Refresh), events![TxList]);
        assert_eq!(lister.state, Some(State::S1B));
    }
}
