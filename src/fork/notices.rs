//! FORK: messages for the user raised where no `App` is at hand.
//!
//! A lifecycle hook's exit status arrives on the thread that waited for it;
//! a `.worktreeinclude` read error happens inside `WorktreeOps::create`.
//! Both used to go to the debug log only. They queue a notice here instead,
//! and the main loop drains the queue every tick into a toast plus a log
//! line (`App::drain_fork_notices`).

use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// Log context, e.g. `hooks` or `worktree`.
    pub context: &'static str,
    pub text: String,
}

static QUEUE: Mutex<Vec<Notice>> = Mutex::new(Vec::new());

/// Queue a warning for the next tick.
pub(crate) fn warn(context: &'static str, text: impl Into<String>) {
    let notice = Notice {
        context,
        text: text.into(),
    };
    // A poisoned lock only means another thread panicked mid-push; the
    // queue itself is still a plain Vec.
    let mut queue = QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    queue.push(notice);
}

/// Everything queued since the last drain, oldest first.
pub(crate) fn drain() -> Vec<Notice> {
    let mut queue = QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

impl crate::app::App {
    /// Toast and log every queued notice. Returns whether anything was shown.
    pub fn drain_fork_notices(&mut self) -> bool {
        let notices = drain();
        if notices.is_empty() {
            return false;
        }
        for notice in notices {
            self.log_warn(notice.context, notice.text.clone());
            self.push_toast_warning(notice.text);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_notices_come_out_once_in_order() {
        let tag = format!("notice-test-{}", std::process::id());
        warn("hooks", format!("{tag} first"));
        warn("worktree", format!("{tag} second"));

        let mine: Vec<Notice> = drain()
            .into_iter()
            .filter(|n| n.text.starts_with(&tag))
            .collect();
        assert_eq!(
            mine,
            vec![
                Notice {
                    context: "hooks",
                    text: format!("{tag} first")
                },
                Notice {
                    context: "worktree",
                    text: format!("{tag} second")
                },
            ]
        );
        assert!(drain().iter().all(|n| !n.text.starts_with(&tag)));
    }
}
