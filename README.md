# empiriqa

[![ci](https://github.com/ynqa/empiriqa/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ynqa/empiriqa/actions/workflows/ci.yml)

Laboratory for pipeline construction with feedback.

![empiriqa.gif](https://github.com/ynqa/ynqa/blob/master/demo/empiriqa.gif)

## Overview

*empiriqa* (command name is `epiq`) is a tool for interactively manipulating
UNIX pipelines `|`. You can individually edit, add, delete, and toggle 
disable/enable for each pipeline stage. It allows you to easily and
efficiently experiment with data processing and analysis using commands.
Additionally, you can execute commands with continuous output streams like `tail -f`.

*empiriqa* can be considered a generalization of tools like
[*jnv*](https://github.com/ynqa/jnv) (interactive JSON filter using jq) and
[*sig*](https://github.com/ynqa/sig) (interactive grep for streaming). While *jnv*
focuses on JSON data manipulation and *sig* specializes in grep searches, *empiriqa*
extends the interactive approach to all UNIX pipeline operations, providing a
more versatile platform for command-line experimentation.

## Installation

### Homebrew

```bash
brew install ynqa/tap/epiq
```

### Cargo

```bash
cargo install epiq

# Or from source (at empiriqa root)
cargo install --path .
```

### Shell

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ynqa/empiriqa/releases/download/v0.1.0/epiq-installer.sh | sh
```

## Usage

```bash
% epiq -h
Laboratory for pipeline construction with feedback

Usage: epiq [OPTIONS]

Options:
      --output-queue-size <OUTPUT_QUEUE_SIZE>
          Set the size of the output queue [default: 1000]
      --event-operate-interval <EVENT_OPERATE_INTERVAL>
          Event processing aggregation interval (milliseconds) [default: 32]
      --output-render-interval <OUTPUT_RENDER_INTERVAL>
          Output rendering interval (milliseconds) [default: 10]
  -h, --help
          Print help (see more with '--help')
  -V, --version
          Print version
```

## Keymap

| Key         | Function                      |
|-------------|-------------------------------|
| `Enter`     | Execute command               |
| `Ctrl+C`    | Exit                          |
| `Esc`       | Toggle mouse capture          |
| `Ctrl+B`    | Add new pipeline stage        |
| `Ctrl+D`    | Delete current pipeline stage |
| `Ctrl+X`    | Disable/Enable current stage  |
| `↑`/`↓`     | Move between stages           |
| `←`/`→`     | Move cursor left/right        |
| `Ctrl+A`    | Move to beginning of line     |
| `Ctrl+E`    | Move to end of line           |
| `Alt+B`     | Move to previous word         |
| `Alt+F`     | Move to next word             |
| `Backspace` | Delete character              |
| `Ctrl+U`    | Clear line                    |
| `Ctrl+W`    | Delete previous word          |
| `Alt+D`     | Delete next word              |

### Enter: Behavior when executing

- When you press Enter key, any currently running command will be interrupted,
  and the new command will be executed
- Error messages such as command execution failures are displayed in red at the
  top
- If you add multiple pipeline stages, the output of each stage is automatically
  passed to the next stage
- Similar to `|&`, both stdout and stderr are automatically processed
- Output can be scrolled using the mouse wheel
- ANSI escape sequences (color and formatting codes) in command output are
  automatically removed and displayed as plain text

### Esc: Toggling mouse capture

By default, *empiriqa* captures all mouse events to provide output scrolling
functionality. This specification means that operations such as text selection
that are normally performed in the terminal are absorbed by the application and
become unavailable.

If you want to select and copy text in the terminal, follow these steps:

1. Press Esc key to disable mouse capture
2. Perform text selection and copying operations
3. If necessary, press Esc key again to re-enable mouse capture

Note: While mouse capture is disabled, you cannot scroll the output.

Technical background:
- The backend uses `crossterm`, and the feature to selectively disable specific
  mouse events is being discussed in the following issue
  - [crossterm#640](https://github.com/crossterm-rs/crossterm/issues/640)

### Ctrl+X: Disabling/Enabling stages

By pressing Ctrl+X, you can toggle the currently selected command stage between
disabled and enabled. Disabled stages are skipped during pipeline execution.
This is useful when you want to temporarily exclude specific commands for
testing.

Disabled stages are displayed with a strikethrough, making them visually
distinguishable.

### Behavior when resizing

When you resize the terminal window, the following automatic adjustments are
made:

- All panels (editor, output, notifications) are re-rendered to fit the screen
  size
- **When height is insufficient**: If the screen height is insufficient for the
  number of pipeline stages, some stages will be automatically deleted
  - Deletion occurs in order from the most recently added stage
  - The main editor (first stage) is not deleted
  - Focus automatically moves to the main editor
- This is an automatic adjustment that differs from shortcut operations
  intentionally performed by the user (such as adding stages with Ctrl+B,
  deleting stages with Ctrl+D, etc.)
- Since stages deleted due to resizing cannot be restored, it is recommended to
  ensure sufficient screen size if you have important editing content

## Limitations

After launching *empiriqa*, commands that require keyboard interaction (such as
`python` and other interactive commands that require input) cannot be executed.
This is because *empiriqa* itself processes keyboard inputs, so commands that
require interactive input in any pipeline stage will not function properly.

## License

This project is licensed under MIT. See [LICENSE](./LICENSE) for details.
