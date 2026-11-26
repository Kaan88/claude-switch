use std::sync::mpsc::{Receiver, Sender, TryRecvError};

type ResultValue = Result<Option<()>, String>;

pub(super) struct Reply<W: Fn()> {
    sender: Option<Sender<ResultValue>>,
    wake: W,
}

impl<W: Fn()> Reply<W> {
    pub(super) fn new(sender: Sender<ResultValue>, wake: W) -> Self {
        Self { sender: Some(sender), wake }
    }

    pub(super) fn complete(mut self, result: ResultValue) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
        }
    }
}

impl<W: Fn()> Drop for Reply<W> {
    fn drop(&mut self) {
        // The picker may drop the callback without running it. Wake only after the message
        // is visible or the sender has disconnected.
        drop(self.sender.take());
        (self.wake)();
    }
}

pub(super) fn take(rx: &Receiver<ResultValue>) -> Option<ResultValue> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("The Save As worker stopped unexpectedly.".into())),
    }
}

#[test]
fn a_disconnected_save_as_wakes_the_dialog_with_a_completed_failure() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let (wake, check_wake) = std::sync::mpsc::channel();
    let reply = Reply::new(sender, || wake.send(()).unwrap());
    assert!(take(&receiver).is_none());
    drop(reply);
    assert_eq!(check_wake.try_recv(), Ok(()));
    assert!(take(&receiver).unwrap().unwrap_err().contains("stopped unexpectedly"));
}
