# Chelotype Backlog

This document tracks known follow-up work that should stay visible in the repo.

## Shell cursor bridge

Chelotype has an internal cursor-target bridge for moving the shell input cursor to an absolute command-line offset without sending many arrow-key escape sequences. This avoids noisy intermediate terminal states on wrapped editable input.

Current state:

- Fish is supported through `commandline -C`.
- Host, Distrobox, Toolbox, and Podman launch targets can carry the Fish bridge.
- Unsupported shells fall back to regular terminal arrow sequences.

Backlog:

- Add a Zsh/ZLE bridge using a custom widget and `CURSOR=N`.
- Investigate whether Bash/readline can support a reliable bridge with `bind -x` and `READLINE_POINT=N`.
- Keep the public Chelotype API shell-agnostic: callers should ask for `write_input_cursor_target(offset)`, while shell-specific modules decide whether they can handle it.
- Add diagnostics that show whether the active pane is using a cursor bridge or fallback arrows.
- Add an end-to-end regression for wrapped long input in a container launch target.

