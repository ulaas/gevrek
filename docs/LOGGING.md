# What gevrek writes to its logs

Every gevrek program starts its log through `gevrek-logging`
(`crates/logging`). Each has a level of its own for everything (the shell
`warn`, the window manager `info`, …); over that, gevrek's own **debug
lines** (every module whose path begins with `gevrek`) are written where
one of these says so, the first that says anything:

1. `RUST_LOG`, where it is set: it alone says, nothing is added
   (`RUST_LOG=info,gevrek_zones=trace`).
2. The setting `desktop.debug_log` (studio: Session → When gevrek starts):
   `on` or `off`. `auto`, the default, leaves it to:
3. The build: `GEVREK_DEBUG_LOG=1 cargo xtask install` writes them in any
   version, `GEVREK_DEBUG_LOG=0` in none.
4. The version: **before 1.0 they are written; from 1.0 on, not.** (A
   pre-release such as `1.2.0-rc.1` counts as before.)

The command `gevrek` writes its debug lines only where the setting (or
`RUST_LOG`) asks for them: its answer, and what a script reads of it, is
not to be lost among them.

Where they are written, a panic is written to the log as well, with the
stack that led to it. The display manager and the greeter read no person's
configuration: for them the build and the version say (or `RUST_LOG` in a
drop-in of `gevrek-dm.service`).

Where the logs are: on Linux, the journal (`journalctl --user -u
gevrek-shell`, `journalctl --user -t gevrek-wl`, `journalctl -u gevrek-dm`);
started with `gevrek start`, the files beside the pid files
(`<runtime folder>/<instance>.<program>.log`), on Windows too.

## Which level

- **error**: something failed that the person will notice, and gevrek could
  not do what was asked.
- **warn**: something unexpected that gevrek got over (a file it could not
  read, a request it refused, a part of the shell that was stuck).
- **info**: what happens a few times in a session: a program starts or
  ends, a display comes or goes, the configuration is applied.
- **debug**: what happened at the edge of a part, one line each time:
  a request came and how it was answered, a program connected or left, a
  file was read or written, a process was started or ended (and how), a
  state changed (shown, hidden, locked, a zone opened), a decision and why
  ("left as it is: the window floats"), an error that is otherwise only
  swallowed (`let _ =`, `.ok()`).
- **trace**: what happens each frame, at each move of the pointer, each
  key, each pass of a loop.

Not excessive: nothing at debug is written in a path that runs each frame
or each event of the pointer. A failure that repeats there is said once,
when it begins (and when it ends), not each time.

## Never in a log

Passwords and anything said to PAM; what is typed and which keys are
pressed (a shortcut's name is fine); what is on the clipboard; the text of
notifications; what is in a person's files. File names and paths are
fine at debug.

## How a line reads

As the lines around it: a sentence in plain words that begins with a small
letter, says what happened to what, and carries the error at its end
(`"the zones' state could not be written to {path}: {error}"`). No emoji.
