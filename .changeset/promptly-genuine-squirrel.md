---
changesette: minor
---

Migrate the interactive prompts of the add command from dialoguer to inquire. Pressing Esc or Ctrl-C during a prompt now cancels the command with exit code 0 instead of killing the process and leaving the cursor hidden.
