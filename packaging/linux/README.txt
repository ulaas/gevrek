gevrek for Linux
================

This archive holds gevrek's files as a package places them on a system
(usr/bin, usr/lib/systemd/user, usr/share, etc/pam.d). It is for a
distribution that takes neither the .deb nor the .rpm of the same release;
where one of those fits, it is the better way: the package manager then
knows the files and removes them again.

To install (as root):

    cp -a usr etc /

etc/pam.d/gevrek-lock is how the lock screen checks your password. It is
written for Fedora and its relatives (it includes system-auth). On a
system that has common-auth instead (Debian and its relatives), its two
lines are:

    @include common-auth
    @include common-account

gevrek is then in the login manager's list of sessions. To try it first as
a window inside the session you are in:

    gevrek-wl

What it needs when it runs, and everything else: usr/share/doc/gevrek/LINUX.md
or https://github.com/ulaas/gevrek/blob/main/docs/LINUX.md

The programs were built on Ubuntu 24.04: they need its glibc (2.39) or a
newer one.

To remove: delete the files this archive holds, by their names in it.
