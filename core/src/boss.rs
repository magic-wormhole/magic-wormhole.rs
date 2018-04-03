use events::Event;
// we process these
use events::Event::{B_Closed, B_Error, B_GotCode, B_GotKey, B_GotMessage,
                    B_GotVerifier, B_Happy, B_RxError, B_RxWelcome, B_Scared};
use events::Event::{API_AllocateCode, API_Close, API_Send, API_SetCode};
// we emit these
use events::Event::RC_Stop;

pub struct Boss {}

impl Boss {
    pub fn new() -> Boss {
        Boss {}
    }
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            API_AllocateCode => vec![],
            API_SetCode(_code) => vec![],
            API_Close => vec![RC_Stop], // eventually signals GotClosed
            API_Send => vec![],
            B_Closed | B_Error | B_GotCode | B_GotKey | B_GotMessage
            | B_GotVerifier | B_Happy | B_RxError | B_RxWelcome | B_Scared => {
                vec![]
            }
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use events::Event::API_Close;

    #[test]
    fn create() {
        let _b = Boss::new();
    }

    fn process_api() {
        let mut b = Boss::new();
        let actions = b.process(API_Close);
        assert_eq!(actions, vec![RC_Stop]);
    }
}
