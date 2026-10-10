# Expiring feedback toasts in agent panes

Captured with the `/aiw-screenshot` workflow from an isolated AIW instance
against a throwaway repository and Claude session. The scenario enters the
live harness, triggers the manual repaint with `Ctrl+Space` then `R`, and waits
past the success toast's three-second lifetime. It does not touch the user's
AIW database or existing tmux sessions.

## 1. Repaint feedback appears as a toast

The repaint confirmation is shown in AIW's standard toast area at the lower
right instead of being written over the harness at the bottom left. The same
promotion applies to any shared AIW status message that reaches pane view.

![Agent pane showing the Repainted agent pane toast](01-repaint-toast.png)

## 2. The toast clears automatically

After the toast expires, AIW redraws the pane without leaving the confirmation
or stale cells behind. The bottom-left harness area remains unobstructed.

![The same agent pane after the repaint toast expires](02-toast-expired.png)
