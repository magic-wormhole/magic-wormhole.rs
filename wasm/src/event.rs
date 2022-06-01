use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Type of an event. Used to determine the fields that are present in the [Event]
pub enum EventType {
    None = 0,
    Progress = 1,
    ServerWelcome = 2,
    FileMetadata = 3,
    ConnectedToRelay = 4,
    Code = 5,
}

impl Default for EventType {
    fn default() -> Self {
        EventType::None
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[wasm_bindgen]
/// Event structure with all possible data fields.
/// Only the fields relevant for the [EventType] will be filled.
pub struct Event {
    event_type: EventType,
    server_welcome_message: String,
    file_name: String,
    file_size: u64,
    progress_current: u64,
    progress_total: u64,
    code: String,
}

impl Event {
    /// Event for a server welcome message (should be shown if not empty).
    pub fn server_welcome(server_welcome_message: String) -> Event {
        Event {
            event_type: EventType::ServerWelcome,
            server_welcome_message,
            ..Event::default()
        }
    }

    /// Event for file metadata (name & size).
    pub fn file_metadata(file_name: String, file_size: u64) -> Event {
        Event {
            event_type: EventType::FileMetadata,
            file_name,
            file_size,
            ..Event::default()
        }
    }

    // Event for establishing the connection to the relay.
    pub fn connected() -> Event {
        Event {
            event_type: EventType::ConnectedToRelay,
            ..Event::default()
        }
    }

    /// Event for the progress of a file transfer (send/receive).
    pub fn progress(progress_current: u64, progress_total: u64) -> Event {
        Event {
            event_type: EventType::Progress,
            progress_current,
            progress_total,
            ..Event::default()
        }
    }

    /// Event for a wormhole code (e.g. 15-foo-bar).
    pub fn code(code: String) -> Event {
        Event {
            event_type: EventType::Code,
            code,
            ..Event::default()
        }
    }
}
