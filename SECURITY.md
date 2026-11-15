# Security

## Reporting a vulnerability

Please report a security problem privately, not in a public issue: use
"Report a vulnerability" on the repository's Security page (a private
advisory). Say which version of gevrek it is (`gevrek --version`), which
system, and how to see the problem.

Only the latest version is supported. gevrek is young and has no older
versions that are mended.

## What gevrek does on your machine

So that you can judge a report, and gevrek itself:

- It listens to the keyboard for its shortcuts, moves and resizes other
  programs' windows, and can start itself when you sign in. On Linux it is
  the compositor: every window's picture and all input pass through it.
- Its programs talk to each other over a local connection that only your
  own programs can open: a named pipe on Windows, a socket in your
  session's runtime folder on Linux. Whoever can open it can do what the
  `gevrek` command can do, which includes starting programs as you. A
  network address is used only when you name one yourself
  (`--address`), and nothing checks who connects there.
- The lock screen and the display manager on Linux ask PAM for your
  password and keep no copy of it.
- It reads and writes its configuration and its state in your own
  folders. It sends nothing anywhere: no account, no telemetry, no check
  for updates.

A problem in any of these is a security problem: a way to reach the
connection from outside your session, a way past the lock screen, a window
or a notification whose content makes gevrek run something.
