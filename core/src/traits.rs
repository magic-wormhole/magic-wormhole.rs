use std::collections::HashMap;

pub enum Action {
    IO(IOEvent),
    API(OutboundAPIEvent),
}

pub trait Core {
    fn start(&mut self) -> Vec<Action>;
    fn execute(&mut self, event: InboundAPIEvent) -> Vec<Action>;

    fn derive_key(&mut self, purpose: &str, length: u8) -> Vec<u8>;
}
