#!/usr/bin/env python3
"""Print N (default 500) random dashboard keys, one per line, in tmux
send-keys spelling. Deterministic (seeded) so a run is repeatable. `q`, `Esc`
and Ctrl+C are left out: those quit on purpose."""
import random
import sys

random.seed(int(sys.argv[2]) if len(sys.argv) > 2 else 7)
count = int(sys.argv[1]) if len(sys.argv) > 1 else 500
pool = list("abcdefghijklmnoprstuvwxyzABCDEFGHIJKLMNOPRSTUVWXYZ0123456789/?.,;:[]{}-=_+!@#$%^&*()") + [
    "Enter", "Tab", "BTab", "Up", "Down", "Left", "Right", "Home", "End", "PPage", "NPage",
    "Space", "BSpace", "DC", "C-j", "C-k", "C-l", "C-u", "C-Space", "C-a", "C-e", "C-r",
    "C-t", "C-w", "C-z", "F1", "F5",
]
for _ in range(count):
    print(random.choice(pool))
