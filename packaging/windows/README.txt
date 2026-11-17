gevrek for Windows
==================

To install, or to put this build over an older one (no administrator needed):

    powershell -ExecutionPolicy Bypass -File install.ps1

The programs go to %LOCALAPPDATA%\Programs\gevrek, that folder goes on your
PATH, and the Start menu gets "gevrek settings", "gevrek (start)" and
"gevrek (stop)". A gevrek that is running is stopped for the update and
started again; the build that was installed before is kept in the folder
"previous" beside the programs. Your configuration is not touched.

Then, in a new terminal:

    gevrek start all            the window manager and the shell
    gevrek enable-autostart     start it when you sign in
    gevrek stop all             stop it; every window is put back
    gevrek-studio               the settings

(The setup program of the same release, gevrek-windows-x64-setup.exe, does
the same with a window to click through.)

Windows of programs that run as an administrator are beyond the reach of a
gevrek that does not. A helper, gevrek-elevated.exe, manages them for it.
To have it started without a question each time, register it once (Windows
asks for an administrator):

    gevrek elevated install     or: install.ps1 -ElevatedHelper, or the
                                setup program's checkbox
    gevrek elevated status      whether it is registered and runs
    gevrek elevated remove      take it away again

A copy of the helper is then kept in %ProgramFiles%\gevrek, where only an
administrator can write. install.ps1 and the setup program bring it to
their version; after `gevrek update` it is the older one until
`gevrek elevated install` is run again. Removing gevrek removes it.

To update:

    gevrek update               get the newest release and install it
    gevrek update --check       only say whether there is one

gevrek also looks by itself: once a day, when it starts, it asks GitHub for
the newest release (nothing is sent but the version of this gevrek), gets a
newer one ready, and installs it the next time it starts. A desktop that is
in use is not interrupted. To turn that off:

    gevrek update --automatic off

Or by hand: unpack the newer package and run its install.ps1.

To remove: Settings, Apps, Installed apps, gevrek; or

    powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\gevrek\uninstall.ps1"

The programs are not signed. Windows may ask before it runs them the first
time, and a virus scanner may look at them twice: gevrek listens to the
keyboard for its shortcuts, moves other programs' windows, and can start
itself at sign-in.
