pub enum MailboxEvent {
    Connected,
    Lost,
    RxMessage,
    RxClosed,
    Close,
    GotMailbox,
    GotMessage,
    AddMessage, // PAKE+VERSION from Key, PHASE from Send
}
