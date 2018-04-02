use events::{InboundEvent, Action};

pub trait Core {
    fn start(&mut self) -> Vec<Action>;
    fn execute(&mut self, event: InboundEvent) -> Vec<Action>;

    fn derive_key(&mut self, purpose: &str, length: u8) -> Vec<u8>;
}
