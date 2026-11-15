#!/usr/bin/env bash
# Install gevrek's boot splash on this machine (resources/plymouth/README.md
# says what it is): the theme for plymouth, made the one in use, and put
# into the initial ramdisk of the kernels that are installed.
#
#     scripts/install-plymouth.sh            install it and say what else
#                                            this machine needs
#     scripts/install-plymouth.sh --check    only say that (changes nothing,
#                                            needs no sudo)
#     scripts/install-plymouth.sh --remove   the theme from before, and
#                                            gevrek's taken away
#
# It asks for sudo where it needs it. Making the ramdisks again takes a
# minute or two. The boot loader and the kernel's command line are not
# touched: --check says what to change there, and you change it.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$REPO/resources/plymouth/gevrek"
THEMES=/usr/share/plymouth/themes
BEFORE=/etc/plymouth/gevrek-theme-before

# What stands between this machine and a start without a seam.
check() {
    local found=0
    say() { found=$((found + 1)); printf '\n  %s\n' "$1"; shift; for line in "$@"; do printf '      %s\n' "$line"; done; }

    echo "From the boot loader to the desktop, on this machine:"
    if ! command -v plymouth >/dev/null 2>&1; then
        say "plymouth is not installed." "sudo dnf install plymouth plymouth-scripts plymouth-plugin-two-step"
    else
        theme="$(plymouth-set-default-theme 2>/dev/null || true)"
        [ "$theme" = gevrek ] || say "The splash in use is '${theme:-none}', not gevrek." "scripts/install-plymouth.sh"
        masked=()
        for unit in plymouth-start plymouth-quit plymouth-quit-wait plymouth-read-write; do
            [ "$(systemctl is-enabled "$unit.service" 2>/dev/null || true)" != masked ] || masked+=("$unit.service")
        done
        [ "${#masked[@]}" = 0 ] || say "The splash is switched off here: its units are masked." \
            "sudo systemctl unmask ${masked[*]}"
    fi

    command_line="$(cat /proc/cmdline)"
    missing=()
    case " $command_line " in *" rhgb "* | *" splash "*) ;; *) missing+=(rhgb) ;; esac
    case " $command_line " in *" quiet "*) ;; *) missing+=(quiet) ;; esac
    [ "${#missing[@]}" = 0 ] || say "The kernel's command line lacks: ${missing[*]} (no splash without rhgb; the kernel's lines on the display without quiet)." \
        "sudo grubby --update-kernel=ALL --args=\"${missing[*]}\""
    case " $command_line " in
    *" console=ttyS"*)
        case " $command_line " in *" plymouth.ignore-serial-consoles "*) ;; *)
            say "The kernel has a serial console: the splash then shows text only." \
                "sudo grubby --update-kernel=ALL --args=plymouth.ignore-serial-consoles" ;;
        esac ;;
    esac

    if [ -e /etc/systemd/system/display-manager.service ]; then
        manager="$(basename "$(readlink -f /etc/systemd/system/display-manager.service)" .service)"
        [ "$manager" = gevrek-dm ] || say "The login is $manager's, not gevrek's: the splash ends as that one ends it." \
            "scripts/install-dm.sh   (and what it prints)"
    else
        say "No login is switched on here: the splash ends in the text console." \
            "scripts/install-dm.sh   (and what it prints)"
    fi
    unit="$(systemctl cat gevrek-dm.service 2>/dev/null || true)"
    if [ -n "$unit" ] && ! grep -q '^Conflicts=.*plymouth-quit.service' <<<"$unit"; then
        say "The installed gevrek-dm is from before it took the splash over itself." "scripts/install-dm.sh"
    fi

    # The boot loader: nothing is seen of it when its menu is hidden and
    # it does not set the display up for itself.
    if [ -r /etc/default/grub ]; then
        grub() { sed -n "s/^$1=//p" /etc/default/grub | tail -1 | tr -d "\"'"; }
        timeout="$(grub GRUB_TIMEOUT)"; style="$(grub GRUB_TIMEOUT_STYLE)"
        if [ "${timeout:-5}" != 0 ] && [ "$style" != hidden ]; then
            say "The boot loader shows its menu for ${timeout:-5} s before every start." \
                "In /etc/default/grub: GRUB_TIMEOUT_STYLE=hidden (the menu is then there on Esc or Shift)," \
                "or GRUB_TIMEOUT=0; then: sudo grub2-mkconfig -o /boot/grub2/grub.cfg"
        fi
        if [ "$(grub GRUB_TERMINAL_OUTPUT)" = gfxterm ]; then
            with=; [ -z "$(grub GRUB_THEME)" ] || with=", with a theme"
            say "The boot loader draws for itself (gfxterm$with): it sets the display's mode and clears it, also with the menu hidden." \
                "In /etc/default/grub: GRUB_TERMINAL_OUTPUT=console (Fedora's own setting; the menu is then plain text)," \
                "then: sudo grub2-mkconfig -o /boot/grub2/grub.cfg"
        fi
    fi

    if [ "$found" = 0 ]; then
        echo "  nothing stands in the way."
    else
        printf '\n%s to change (each is yours to decide; nothing was changed here).\n' "$found"
    fi
}

case "${1:-}" in
--check)
    check
    exit 0 ;;
--remove)
    [ "$(id -u)" != 0 ] || { echo "Run it as yourself: it asks for sudo where it needs it." >&2; exit 1; }
    before="$(cat "$BEFORE" 2>/dev/null || true)"
    # (A theme that is gone meanwhile: the system's own choice.)
    [ -n "$before" ] && [ -d "$THEMES/$before" ] || before=
    if [ "$(plymouth-set-default-theme 2>/dev/null || true)" = gevrek ]; then
        echo "== the splash from before (${before:-as the system has it}), and the ramdisks made again (sudo)"
        if [ -n "$before" ]; then
            sudo plymouth-set-default-theme "$before"
        else
            sudo plymouth-set-default-theme --reset
        fi
        sudo dracut --regenerate-all --force
    fi
    sudo rm -rf "$THEMES/gevrek" "$BEFORE"
    echo "Removed."
    exit 0 ;;
"") ;;
*)
    echo "scripts/install-plymouth.sh [--check | --remove]" >&2
    exit 1 ;;
esac

[ "$(id -u)" != 0 ] || { echo "Run it as yourself: it asks for sudo where it needs it." >&2; exit 1; }
command -v plymouth-set-default-theme >/dev/null 2>&1 || {
    echo "plymouth is not here: sudo dnf install plymouth plymouth-scripts plymouth-plugin-two-step" >&2
    exit 1
}
plugins="$(plymouth --get-splash-plugin-path 2>/dev/null || echo /usr/lib64/plymouth/)"
[ -f "${plugins%/}/two-step.so" ] || {
    echo "plymouth's two-step module is not here: sudo dnf install plymouth-plugin-two-step" >&2
    exit 1
}
[ -f "$SOURCE/gevrek.plymouth" ] && [ -f "$SOURCE/throbber-0001.png" ] || {
    echo "the theme's files are not in $SOURCE (scripts/make-plymouth-theme.py draws the pictures)" >&2
    exit 1
}

echo "== installing the theme (sudo)"
now="$(plymouth-set-default-theme 2>/dev/null || true)"
if [ -n "$now" ] && [ "$now" != gevrek ]; then
    echo "$now" | sudo tee "$BEFORE" >/dev/null
fi
sudo rm -rf "$THEMES/gevrek"
sudo install -d -m 0755 "$THEMES/gevrek"
sudo install -m 0644 "$SOURCE"/gevrek.plymouth "$SOURCE"/*.png "$THEMES/gevrek/"
if command -v restorecon >/dev/null 2>&1; then
    sudo restorecon -R "$THEMES/gevrek" 2>/dev/null || true
fi
echo "== making it the splash in use, and the ramdisks again (a minute or two)"
sudo plymouth-set-default-theme gevrek
sudo dracut --regenerate-all --force

cat <<TEXT

Installed: gevrek's splash is shown from the next start${now:+ (before: $now; --remove brings it back)}.

TEXT
check
