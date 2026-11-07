//! Metadata-only debug output with bounded deferral while a TUI owns stderr.

use std::{
    cell::RefCell,
    collections::VecDeque,
    fmt,
    io::{self, Write},
    marker::PhantomData,
    rc::Rc,
};

const MAX_DEFERRED_MESSAGES: usize = 256;
#[derive(Default)]
struct Buffer {
    messages: VecDeque<String>,
    dropped: usize,
}
thread_local! {
    static DEFERRED: RefCell<Option<Buffer>> = const { RefCell::new(None) };
}

/// Preserve normal CLI diagnostics; a scoped interactive owner queues them.
pub(crate) fn log(args: fmt::Arguments<'_>) {
    if crate::service::debug_enabled() {
        emit(&format!("[rtk-debug] {args}"), &mut io::stderr().lock());
    }
}

fn emit(message: &str, output: &mut impl Write) {
    let queued = DEFERRED.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(buffer) = slot.as_mut() else {
            return false;
        };
        if buffer.messages.len() == MAX_DEFERRED_MESSAGES {
            buffer.messages.pop_front();
            buffer.dropped += 1;
        }
        buffer.messages.push_back(message.to_string());
        true
    });
    if !queued {
        // Diagnostics cannot turn a successful command into an I/O failure.
        let _ = writeln!(output, "{message}");
    }
}

/// Drop only after leaving raw mode and the alternate screen. Thread-local so
/// another thread's CLI activity cannot change this terminal's logging policy.
#[must_use]
pub(crate) fn defer() -> DeferredDebug {
    let previous = DEFERRED.with(|slot| slot.replace(Some(Buffer::default())));
    DeferredDebug {
        previous,
        active: true,
        same_thread: PhantomData,
    }
}

pub(crate) struct DeferredDebug {
    previous: Option<Buffer>,
    active: bool,
    same_thread: PhantomData<Rc<()>>,
}
impl DeferredDebug {
    fn flush_to(&mut self, output: &mut impl Write) {
        if !self.active {
            return;
        }
        self.active = false;
        let buffer = DEFERRED.with(|slot| slot.replace(self.previous.take()));
        if let Some(buffer) = buffer {
            if buffer.dropped > 0 {
                emit(
                    &format!(
                        "[rtk-debug] debug_log dropped={} retained={} limit={MAX_DEFERRED_MESSAGES}",
                        buffer.dropped,
                        buffer.messages.len()
                    ),
                    output,
                );
            }
            for message in buffer.messages {
                emit(&message, output);
            }
        }
    }
}
impl Drop for DeferredDebug {
    fn drop(&mut self) {
        self.flush_to(&mut io::stderr().lock());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_events_write_nothing_until_the_screen_owner_flushes() {
        let mut output = Vec::new();
        let mut guard = defer();
        for event in [
            "tab",
            "sort",
            "scroll",
            "resize",
            "refresh",
            "tracking.query",
            "hook_check",
        ] {
            emit(event, &mut output);
            assert!(output.is_empty(), "external terminal bytes during {event}");
        }
        guard.flush_to(&mut output);
        assert_eq!(String::from_utf8(output).unwrap().lines().count(), 7);
    }

    #[test]
    fn long_sessions_bound_memory_and_report_omitted_diagnostics() {
        let mut output = Vec::new();
        let mut guard = defer();
        for i in 0..MAX_DEFERRED_MESSAGES + 5 {
            emit(&format!("event={i}"), &mut output);
        }
        DEFERRED.with(|slot| {
            let slot = slot.borrow();
            let buffer = slot.as_ref().unwrap();
            assert_eq!(buffer.messages.len(), MAX_DEFERRED_MESSAGES);
            assert_eq!(buffer.dropped, 5);
        });
        guard.flush_to(&mut output);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("dropped=5"));
        assert!(output.lines().nth(1).unwrap().starts_with("event=5"));
    }

    #[test]
    fn nested_owners_restore_the_previous_destination_without_early_output() {
        let mut output = Vec::new();
        let mut outer = defer();
        emit("outer", &mut output);
        let mut inner = defer();
        emit("inner", &mut output);
        inner.flush_to(&mut output);
        assert!(output.is_empty());
        outer.flush_to(&mut output);
        assert_eq!(String::from_utf8(output).unwrap(), "outer\ninner\n");
    }

    #[test]
    fn noninteractive_diagnostics_still_write_immediately() {
        let mut output = Vec::new();
        emit("plain", &mut output);
        assert_eq!(output, b"plain\n");
    }
}
