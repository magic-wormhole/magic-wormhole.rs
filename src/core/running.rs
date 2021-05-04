use super::{key, mailbox};
use crate::{core::*, APIEvent};
use std::collections::VecDeque;

#[derive(Debug, derive_more::Display)]
#[display(
    fmt = "RunningMachine {{ phase: {}, side: {}, await_nameplate_release: {}, mailbox_machine: {}",
    phase,
    side,
    await_nameplate_release,
    mailbox_machine
)]
pub(super) struct RunningMachine {
    pub phase: u64,
    pub key: xsalsa20poly1305::Key,
    pub side: MySide,
    pub await_nameplate_release: bool,
    pub mailbox_machine: mailbox::MailboxMachine,
}

impl RunningMachine {
    pub(super) fn send_message(
        mut self: Box<Self>,
        actions: &mut VecDeque<Event>,
        plaintext: Vec<u8>,
    ) -> super::State {
        let phase_string = Phase(format!("{}", self.phase));
        let data_key = key::derive_phase_key(&self.side, &self.key, &phase_string);
        let (_nonce, encrypted) = key::encrypt_data(&data_key, &plaintext);
        self.mailbox_machine
            .send_message(actions, phase_string, encrypted);
        self.phase += 1;

        super::State::Running(self)
    }

    pub(super) fn receive_message(
        mut self: Box<Self>,
        actions: &mut VecDeque<Event>,
        message: EncryptedMessage,
    ) -> super::State {
        if !self.mailbox_machine.receive_message(&message) {
            return super::State::Running(self);
        }
        if message.phase.to_num().is_none() {
            // TODO: log and ignore, for future expansion
            todo!("log and ignore, for future expansion");
        }

        // TODO maybe reorder incoming messages by phase numeral?
        match message.decrypt(&self.key) {
            Some(plaintext) => {
                actions.push_back(APIEvent::GotMessage(plaintext).into());
            },
            None => {
                actions.push_back(Event::ShutDown(Err(WormholeCoreError::Crypto)));
            },
        }

        super::State::Running(self)
    }

    pub(super) fn shutdown(
        self: Box<Self>,
        actions: &mut VecDeque<Event>,
        result: Result<(), WormholeCoreError>,
    ) -> super::State {
        self.mailbox_machine.close(
            actions,
            match &result {
                Ok(_) => Mood::Happy,
                Err(e) if e.is_scared() => Mood::Scared,
                Err(_) => Mood::Errory,
            },
        );
        super::State::Closing {
            await_nameplate_release: self.await_nameplate_release,
            await_mailbox_close: true,
            result,
        }
    }
}
