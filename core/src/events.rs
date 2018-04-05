use std::collections::HashMap;
// Events come into the core, Actions go out of it (to the IO glue layer)
use api::{APIAction, APIEvent, IOAction, IOEvent, Mood, TimerHandle, WSHandle};

#[derive(Debug, PartialEq)]
pub enum Machine {
    API_Action,
    IO_Action,
    Allocator,
    Boss,
    Code,
    Input,
    Key,
    Lister,
    Mailbox,
    Nameplate,
    Order,
    Receive,
    Rendezvous,
    Send,
    Terminator,
}

// machines (or IO, or the API) emit these events, and each is routed to a
// specific machine (or IO or the API)

#[derive(Debug, PartialEq)]
pub enum Event {
    // API events (instructions from the application)
    API_AllocateCode,
    API_SetCode(String),
    API_Close,
    API_Send(Vec<u8>),
    // API Actions (results sent back to the application)
    API_GotWelcome(HashMap<String, String>), // anything JSON-able: Value?
    API_GotCode(String), // must be easy to canonically encode into UTF-8 bytes
    API_GotUnverifiedKey(Vec<u8>),
    API_GotVerifier(Vec<u8>),
    API_GotVersions(HashMap<String, String>), // actually anything JSON-able
    API_GotMessage(Vec<u8>),
    API_GotClosed(Mood),
    // IO actions (instructions sent to our external IO glue layer)
    IO_StartTimer(TimerHandle, f32),
    IO_CancelTimer(TimerHandle),
    IO_WebSocketOpen(WSHandle, String), // url
    IO_WebSocketSendMessage(WSHandle, String),
    IO_WebSocketClose(WSHandle),
    // IO events (responses from the glue layer)
    IO_TimerExpired(TimerHandle),
    IO_WebSocketConnectionMade(WSHandle),
    IO_WebSocketMessageReceived(WSHandle, String),
    IO_WebSocketConnectionLost(WSHandle),

    // A is for Allocator
    A_Connected,
    A_Lost,
    A_RxAllocated,
    // B is for Boss
    B_RxWelcome,
    B_RxError,
    B_Error,
    B_Closed,
    B_GotCode(String),
    B_GotKey(Vec<u8>), // TODO: fixed length?
    B_Scared,
    B_Happy,
    B_GotVerifier(Vec<u8>), // TODO: fixed length (sha256)
    B_GotMessage(String, String, Vec<u8>),
    // C is for Code
    C_AllocateCode,
    C_InputCode,
    C_SetCode(String),
    C_Allocated,
    C_GotNameplate,
    C_FinishedInput,
    // I is for Input
    I_Start,
    I_GotNameplates,
    I_GotWordlist,
    // K is for Key
    K_GotPake,
    K_GotMessage,
    // L is for Lister
    L_Connected,
    L_Lost,
    L_RxNameplates,
    L_Refresh,
    // M is for Mailbox
    M_Connected,
    M_Lost,
    M_RxMessage,
    M_RxClosed,
    M_Close,
    M_GotMailbox,
    M_GotMessage,
    M_AddMessage, // PAKE+VERSION from Key, PHASE from Send
    // N is for Nameplate
    N_NameplateDone,
    N_Connected,
    N_Lost,
    N_RxClaimed,
    N_RxReleased,
    N_SetNameplate(String),
    N_Release,
    // O is for Order
    O_GotMessage,
    // R is for Receive
    R_GotCode,
    R_GotKey,
    // RC is for Rendezvous
    RC_Start,
    RC_TxOpen,
    RC_TxAdd,
    RC_TxClose,
    RC_Stop,
    RC_TxClaim,
    RC_TxRelease,
    RC_TxAllocate,
    RC_TxList,
    // S is for Send
    S_Send(Vec<u8>),
    S_GotVerifiedKey,
    // T is for Terminator
    T_Close(Mood),
    T_MailboxDone,
    T_NameplateDone,
    T_Stopped,
}

pub enum AllocatorEvent {
    Connected,
    Lost,
    RxAllocated,
}

// and a flat-to-(machine, machine-specific-event) mapping function

pub fn machine_for_event(e: &Event) -> Machine {
    use self::Event::*;
    use self::Machine::*;
    match e {
        &API_AllocateCode | &API_SetCode(_) | &API_Close | &API_Send(_) => {
            API_Action
        }
        &API_GotWelcome(_)
        | &API_GotCode(_)
        | &API_GotUnverifiedKey(_)
        | &API_GotVerifier(_)
        | &API_GotVersions(_)
        | &API_GotMessage(_)
        | &API_GotClosed(_) => Boss,
        &IO_StartTimer(_, _)
        | &IO_CancelTimer(_)
        | &IO_WebSocketOpen(_, _)
        | &IO_WebSocketSendMessage(_, _)
        | &IO_WebSocketClose(_) => IO_Action,
        &IO_TimerExpired(_)
        | &IO_WebSocketConnectionMade(_)
        | &IO_WebSocketMessageReceived(_, _)
        | &IO_WebSocketConnectionLost(_) => Rendezvous, // IO currently goes RC
        &A_Connected | &A_Lost | &A_RxAllocated => Allocator,
        &B_RxWelcome
        | &B_RxError
        | &B_Error
        | &B_Closed
        | &B_GotCode(_)
        | &B_GotKey(_)
        | &B_Scared
        | &B_Happy
        | &B_GotVerifier(_)
        | &B_GotMessage(_, _, _) => Boss,
        &C_AllocateCode | &C_InputCode | &C_SetCode(_) | &C_Allocated
        | &C_GotNameplate | &C_FinishedInput => Code,
        &I_Start | &I_GotNameplates | &I_GotWordlist => Input,
        &K_GotPake | &K_GotMessage => Key,
        &L_Connected | &L_Lost | &L_RxNameplates | &L_Refresh => Lister,
        &M_Connected | &M_Lost | &M_RxMessage | &M_RxClosed | &M_Close
        | &M_GotMailbox | &M_GotMessage | &M_AddMessage => Mailbox,
        &N_NameplateDone | &N_Connected | &N_Lost | &N_RxClaimed
        | &N_RxReleased | &N_SetNameplate(_) | &N_Release => Nameplate,
        &O_GotMessage => Order,
        &R_GotCode | &R_GotKey => Receive,
        &RC_Start | &RC_TxOpen | &RC_TxAdd | &RC_TxClose | &RC_Stop
        | &RC_TxClaim | &RC_TxRelease | &RC_TxAllocate | &RC_TxList => {
            Rendezvous
        }
        &S_Send(_) | &S_GotVerifiedKey => Send,
        &T_Close(_) | &T_MailboxDone | &T_NameplateDone | &T_Stopped => {
            Terminator
        }
    }
}

impl From<APIEvent> for Event {
    fn from(r: APIEvent) -> Self {
        use APIEvent::*;
        use self::Event::*;
        match r {
            AllocateCode => API_AllocateCode,
            SetCode(code) => API_SetCode(code),
            Close => API_Close,
            Send(plaintext) => API_Send(plaintext),
        }
    }
}

impl From<Event> for APIAction {
    fn from(r: Event) -> Self {
        use self::Event::*;
        use APIAction::*;
        match r {
            API_GotWelcome(data) => GotWelcome(data),
            API_GotCode(code) => GotCode(code),
            API_GotUnverifiedKey(key) => GotUnverifiedKey(key),
            API_GotVerifier(verifier) => GotVerifier(verifier),
            API_GotVersions(data) => GotVersions(data),
            API_GotMessage(message) => GotMessage(message),
            API_GotClosed(result) => GotClosed(result),
            _ => panic!(),
        }
    }
}

impl From<Event> for IOAction {
    fn from(r: Event) -> Self {
        use self::Event::*;
        use IOAction::*;
        match r {
            IO_StartTimer(handle, duration) => StartTimer(handle, duration),
            IO_CancelTimer(handle) => CancelTimer(handle),
            IO_WebSocketOpen(handle, url) => WebSocketOpen(handle, url),
            IO_WebSocketSendMessage(handle, message) => {
                WebSocketSendMessage(handle, message)
            }
            IO_WebSocketClose(handle) => WebSocketClose(handle),
            _ => panic!(),
        }
    }
}

impl From<IOEvent> for Event {
    fn from(r: IOEvent) -> Self {
        use IOEvent::*;
        use self::Event::*;
        match r {
            TimerExpired(handle) => IO_TimerExpired(handle),
            WebSocketConnectionMade(handle) => {
                IO_WebSocketConnectionMade(handle)
            }
            WebSocketMessageReceived(handle, message) => {
                IO_WebSocketMessageReceived(handle, message)
            }
            WebSocketConnectionLost(handle) => {
                IO_WebSocketConnectionLost(handle)
            }
        }
    }
}
