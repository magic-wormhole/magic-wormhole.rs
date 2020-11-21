use super::key;
use super::mailbox;
use super::Key;
use crate::core::EncryptedMessage;
use crate::core::Event;
use crate::core::Mood;
use crate::core::MySide;
use crate::core::Phase;
use crate::APIEvent;
use std::collections::VecDeque;

pub(super) struct RunningMachine {
    pub phase: u64,
    pub key: Key,
    pub side: MySide,
    pub await_nameplate_release: bool,
    pub mailbox_machine: mailbox::MailboxMachine,
}

impl RunningMachine {
    pub(super) fn send_message(
        mut self,
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
        mut self,
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
            Ok(plaintext) => {
                actions.push_back(APIEvent::GotMessage(plaintext).into());
            },
            Err(error) => {
                actions.push_back(Event::ShutDown(Err(error)));
            },
        }

        super::State::Running(self)
    }

    pub(super) fn shutdown(
        self,
        actions: &mut VecDeque<Event>,
        result: anyhow::Result<()>,
    ) -> super::State {
        // TODO handle "scared" mood
        self.mailbox_machine.close(
            actions,
            if result.is_ok() {
                Mood::Happy
            } else {
                Mood::Errory
            },
        );
        super::State::Closing {
            await_nameplate_release: self.await_nameplate_release,
            await_mailbox_close: true,
            result,
        }
    }
}
