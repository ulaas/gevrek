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

To update: unpack the newer package and run its install.ps1.

To remove: Settings, Apps, Installed apps, gevrek; or

    powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\gevrek\uninstall.ps1"

The programs are not signed. Windows may ask before it runs them the first
time, and a virus scanner may look at them twice: gevrek listens to the
keyboard for its shortcuts, moves other programs' windows, and can start
itself at sign-in.
