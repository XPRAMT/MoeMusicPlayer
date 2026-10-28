//! Ending a file read from outside (the album detail page leaves while its cover scan is reading a file): the thread that
//! reads registers a flag for the duration of a piece of work, and the readers underneath (the remote range reader, the
//! disk cache) look at it before each request to a server and stop with an error once it is set (not `Interrupted`: the standard library retries a read that fails with it).

use std::cell::RefCell;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

thread_local! {
    static READ_CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Run `work` on this thread with `flag` as the cancel flag of every read it makes.
pub fn with_read_cancel<T>(flag: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
    let previous = READ_CANCEL.with(|slot| slot.borrow_mut().replace(flag));
    let result = work();
    READ_CANCEL.with(|slot| *slot.borrow_mut() = previous);
    result
}

/// Whether the work this thread runs was cancelled (always `false` outside [`with_read_cancel`]).
pub fn read_cancelled() -> bool {
    READ_CANCEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    })
}

/// The error a read returns once it was cancelled.
pub fn read_cancelled_error() -> io::Error {
    io::Error::other("the read was cancelled")
}

/// `Err` when the work of this thread was cancelled: the check a reader makes before a request to a server.
pub fn check_read_cancel() -> io::Result<()> {
    if read_cancelled() {
        Err(read_cancelled_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_seen_only_inside_the_work_and_only_when_set() {
        assert!(!read_cancelled());
        let flag = Arc::new(AtomicBool::new(false));
        with_read_cancel(Arc::clone(&flag), || {
            assert!(check_read_cancel().is_ok());
            flag.store(true, Ordering::SeqCst);
            assert_eq!(
                check_read_cancel().unwrap_err().kind(),
                io::ErrorKind::Other
            );
        });
        assert!(!read_cancelled(), "the flag does not outlive the work");
    }
}
