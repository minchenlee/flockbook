//! Ordered image submission. A timeout leaves the agent input untouched; never guess an acknowledgement.
use crate::{image_paste, AgentIdentity};
use std::time::{Duration, Instant};
pub trait SendIO {
    /// Must check identity, current input permission and absence of a picker immediately before every write.
    fn gate(&self, expect: &AgentIdentity) -> Result<(), String>;
    fn screen(&self) -> Result<String, String>;
    fn input(&self, text: &str) -> Result<(), String>;
    fn paste(&self, text: &str) -> Result<(), String>;
    fn enter(&self) -> Result<(), String>;
    fn settle(&self) {
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn send(
    io: &impl SendIO,
    expect: &AgentIdentity,
    text: &str,
    images: &[String],
    max_wait: Duration,
) -> Result<(), String> {
    // Validate every path before the first mutation.
    let pasted: Vec<_> = images
        .iter()
        .map(|p| image_paste::paste(p))
        .collect::<Result<_, _>>()?;
    let wait = !images.is_empty() && image_paste::waits_for_marker(&expect.kind);
    io.gate(expect)?;
    let target = if wait {
        image_paste::input_markers(&io.screen()?, &expect.kind) + images.len()
    } else {
        0
    };
    if !text.is_empty() {
        io.gate(expect)?;
        io.input(text)?;
    }
    for path in pasted {
        io.gate(expect)?;
        io.paste(&path)?;
    }
    let ready = || -> Result<(), String> {
        io.gate(expect)?;
        if wait && image_paste::input_markers(&io.screen()?, &expect.kind) < target {
            return Err("Image is not ready; check the agent input before retrying".into());
        }
        Ok(())
    };
    if wait {
        let deadline = Instant::now() + max_wait;
        loop {
            io.settle();
            io.gate(expect)?;
            if image_paste::input_markers(&io.screen()?, &expect.kind) >= target {
                break;
            }
            if Instant::now() >= deadline {
                return Err(
                    "Image acknowledgement timed out; message remains in the agent input".into(),
                );
            }
        }
    }
    ready()?;
    io.enter()?;
    if wait {
        // Some agents swallow the first Enter while converting the attachment. Retry once, only while markers persist.
        for _ in 0..5 {
            io.settle();
        }
        if image_paste::input_markers(&io.screen()?, &expect.kind) >= target {
            ready()?;
            io.enter()?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};
    struct IO {
        screens: RefCell<VecDeque<String>>,
        calls: RefCell<Vec<String>>,
    }
    impl SendIO for IO {
        fn gate(&self, _: &AgentIdentity) -> Result<(), String> {
            self.calls.borrow_mut().push("gate".into());
            Ok(())
        }
        fn screen(&self) -> Result<String, String> {
            let mut s = self.screens.borrow_mut();
            Ok(if s.len() > 1 {
                s.pop_front().unwrap()
            } else {
                s.front().cloned().unwrap_or_default()
            })
        }
        fn input(&self, t: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("text:{t}"));
            Ok(())
        }
        fn paste(&self, _: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("paste".into());
            Ok(())
        }
        fn enter(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("enter".into());
            Ok(())
        }
        fn settle(&self) {}
    }
    fn identity() -> AgentIdentity {
        AgentIdentity {
            pane_id: "p".into(),
            name: "agent".into(),
            kind: "claude".into(),
            session_id: Some("s".into()),
        }
    }
    #[test]
    fn no_enter_without_new_marker() {
        let io = IO {
            screens: RefCell::new(["❯ [Image #1]".into()].into()),
            calls: RefCell::default(),
        };
        assert!(send(
            &io,
            &identity(),
            "  exact\n\n",
            &["/tmp/image.png".into()],
            Duration::ZERO
        )
        .is_err());
        assert!(!io.calls.borrow().iter().any(|c| c == "enter"));
        assert!(io.calls.borrow().iter().any(|c| c == "text:  exact\n\n"));
    }
    #[test]
    fn paste_ack_precedes_enter() {
        let io = IO {
            screens: RefCell::new(
                [
                    "❯".into(),
                    "❯".into(),
                    "❯ [Image #1]".into(),
                    "❯ [Image #1]".into(),
                    "❯".into(),
                ]
                .into(),
            ),
            calls: RefCell::default(),
        };
        send(
            &io,
            &identity(),
            "",
            &["/tmp/image.png".into()],
            Duration::from_secs(1),
        )
        .unwrap();
        let calls = io.calls.borrow();
        assert!(calls.iter().position(|c| c == "paste") < calls.iter().position(|c| c == "enter"));
        assert_eq!(calls.iter().filter(|c| *c == "enter").count(), 1);
    }
}
