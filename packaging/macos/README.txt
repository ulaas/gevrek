gevrek for macOS
================

Build and install (for this user; no administrator needed):

    packaging/macos/build-app.sh      -> target/macos/Gevrek.app, signed
    packaging/macos/install.sh        -> ~/Applications/Gevrek.app, and
                                         gevrek, gevrek-studio in ~/.local/bin

Then:

    gevrek start wm             the window manager
    gevrek start shell          the panel, the launcher and the rest of the
                                shell (in the room the window manager keeps)
    gevrek enable-autostart     start it when you sign in (a LaunchAgent)
    gevrek stop wm              stop it; every window is put back
    gevrek-studio               the settings

The first time, macOS asks to allow gevrek in System Settings, Privacy &
Security, Accessibility. Allow it, and start gevrek again. The permission
is the bundle's (io.github.ulaas.gevrek, with its signature's team), so a
newer build signed the same way keeps it.

The keys of the bindings: gevrek's Win key is Option, its Alt is
Control+Option+Command, Ctrl is Control. Nothing is on Command alone, so
copy, paste and quit stay every program's. With Option held, the left
button moves the window under the pointer and the right button resizes it
(GEVREK_MACOS_DRAG_MODIFIER names another modifier, or `none`).

A letter of a binding is the key that writes it in the keyboard layout
in use (Win+Z is the key marked Z on a German keyboard too); the other
keys are named by their place.

Touchpad gestures (desktop.gestures) read the fingers through macOS's
MultitouchSupport. macOS's own gestures go on beside them: give gevrek's
profiles four fingers, or turn macOS's off in System Settings, Trackpad,
More Gestures.

What macOS asks before the shell has it: pictures of windows (for the
switcher) and of the menu bar's items (the tray) need Screen Recording;
the tray reads and presses the menu bar's items with Accessibility; the
wifi's networks are named only once gevrek may know the location. Each is
asked for once, in Gevrek's name. What plays is not shown: macOS tells it
only to Apple's own programs.

To try gevrek on a few programs only, without it taking over the
desktop, name their bundle ids:

    GEVREK_MACOS_ONLY=com.apple.activitymonitor,com.apple.dictionary gevrek start wm

To remove it: packaging/macos/uninstall.sh

Signing: build-app.sh signs with GEVREK_SIGN_IDENTITY, or else the first
"Developer ID Application" or "Apple Development" identity of the
keychain, or else ad hoc (which works, but the permission is asked for
again after every build). A bundle for others is to be notarized
(xcrun notarytool) after it is built.
