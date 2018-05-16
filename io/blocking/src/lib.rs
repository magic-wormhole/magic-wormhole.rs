extern crate magic_wormhole_core;
use magic_wormhole_core::WormholeCore;

pub struct Wormhole {
    core: WormholeCore,
}

impl Wormhole {
    pub fn new(appid: &str, relay_url: &str) -> Wormhole {
        let mut w = Wormhole { core: WormholeCore::new(appid, relay_url) };
        w.core.start();
        w
    }

    pub fn set_code(&mut self, code: &str) {
    }

    pub fn send_message(&mut self, msg: &[u8]) {
    }

    pub fn get_message(&mut self) -> Vec<u8> {
        b"fake".to_vec()
    }

    pub fn close(&mut self) { // TODO mood
    }
}
