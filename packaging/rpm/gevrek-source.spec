# gevrek as an RPM built from source, for a repository (COPR).
#
# `gevrek.spec` beside this file packs programs that are built already;
# this one builds them. The crates gevrek depends on are fetched by cargo
# while it builds, so the build needs the network: in COPR that is
# "Enable internet access during builds" in the project's settings.
#
#   rpmbuild -bs --define "gevrek_commit <the whole commit>" gevrek-source.spec
#   (or, in COPR, "Build from SCM" with this file as the spec)
#
# Without `gevrek_commit` the branch `master` is built.

%{!?gevrek_commit: %global gevrek_commit master}
%global gevrek_short %(echo %{gevrek_commit} | cut -c1-9)

Name:           gevrek
Version:        0.7.6
Release:        1.%{gevrek_short}%{?dist}
Summary:        A tiling window manager and desktop for Wayland
License:        MIT
URL:            https://github.com/ulaas/gevrek
Source0:        %{url}/archive/%{gevrek_commit}/gevrek-%{gevrek_commit}.tar.gz

# Rust's own debug information is left out: the programs are large enough.
%global debug_package %{nil}

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  clang
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(libudev)
BuildRequires:  pkgconfig(libinput)
BuildRequires:  pkgconfig(libseat)
BuildRequires:  pkgconfig(gbm)
BuildRequires:  pkgconfig(libdrm)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(pixman-1)
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(wayland-server)
BuildRequires:  pkgconfig(egl)
BuildRequires:  pkgconfig(libsystemd)
BuildRequires:  pkgconfig(libpipewire-0.3)
BuildRequires:  pam-devel
BuildRequires:  systemd-rpm-macros

Requires:       polkit
Requires:       mesa-dri-drivers
Recommends:     xdg-desktop-portal
Recommends:     xdg-desktop-portal-gtk
Recommends:     xorg-x11-server-Xwayland
Recommends:     pipewire-utils
Recommends:     foot
# For gevrek's boot splash, where one is wanted.
Suggests:       plymouth-plugin-two-step

%description
gevrek is a tiling Wayland compositor with its own desktop around it: a
panel, a launcher, notifications, a lock screen, an authentication agent,
and a settings program (gevrek-studio) in which every setting has a form.
After installing, gevrek is in the login manager's list of sessions.

%prep
%autosetup -n gevrek-%{gevrek_commit}

%build
# The archive is no checkout: the build is told which commit it is.
export GEVREK_COMMIT=%{gevrek_short}
cargo build --release --locked --bins \
    -p gevrek -p gevrek-smithay -p gevrek-studio -p gevrek-shell -p gevrek-lock \
    -p gevrek-dm

%install
install -d %{buildroot}%{_bindir} %{buildroot}%{_userunitdir}
for program in gevrek gevrek-wl gevrek-studio gevrek-shell gevrek-lock gevrek-dm; do
    install -m 0755 target/release/$program %{buildroot}%{_bindir}/$program
done
install -m 0755 resources/gevrek-session %{buildroot}%{_bindir}/gevrek-session
# The units name the programs by their place.
for unit in gevrek.service gevrek-safe.service gevrek-shell.service gevrek-shutdown.target; do
    sed 's|@BIN@|%{_bindir}|g' resources/systemd/$unit > %{buildroot}%{_userunitdir}/$unit
    chmod 0644 %{buildroot}%{_userunitdir}/$unit
done
install -D -m 0644 resources/gevrek.desktop %{buildroot}%{_datadir}/wayland-sessions/gevrek.desktop
install -D -m 0644 resources/gevrek-studio.desktop %{buildroot}%{_datadir}/applications/gevrek-studio.desktop
# gevrek's icon, by the name the application's entry gives (`Icon=gevrek`).
for size in 16 24 32 48 64 128 256 512; do
    install -D -m 0644 assets/brand/app-icon/linux/hicolor/${size}x${size}/apps/gevrek.png %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/gevrek.png
done
install -D -m 0644 assets/brand/app-icon/linux/hicolor/scalable/apps/gevrek.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/gevrek.svg
install -D -m 0644 resources/gevrek-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/gevrek-portals.conf
install -D -m 0644 resources/gevrek.portal %{buildroot}%{_datadir}/xdg-desktop-portal/portals/gevrek.portal
# The lock screen checks the password as every login on the system does.
install -d %{buildroot}%{_sysconfdir}/pam.d
cat > %{buildroot}%{_sysconfdir}/pam.d/gevrek-lock <<'PAM'
#%PAM-1.0
auth       include      system-auth
account    include      system-auth
PAM
# gevrek's own display manager (resources/dm/README.md). Installed, not
# enabled: `systemctl enable gevrek-dm` in place of the one the machine has.
install -D -m 0644 resources/dm/gevrek-dm.service %{buildroot}%{_unitdir}/gevrek-dm.service
install -D -m 0644 resources/dm/pam-gevrek-dm %{buildroot}%{_sysconfdir}/pam.d/gevrek-dm
install -D -m 0644 resources/dm/pam-gevrek-greeter %{buildroot}%{_sysconfdir}/pam.d/gevrek-greeter
install -D -m 0644 resources/dm/sysusers.conf %{buildroot}%{_sysusersdir}/gevrek-dm.conf
install -D -m 0644 resources/dm/tmpfiles.conf %{buildroot}%{_tmpfilesdir}/gevrek-dm.conf
# The SELinux module that labels gevrek-dm as a display manager (without it
# the daemon runs as a service nobody confines, and the session keeps that
# label): put where the policy's packages lie, and installed below.
install -D -m 0644 resources/dm/gevrek-dm.cil %{buildroot}%{_datadir}/selinux/packages/gevrek-dm.cil
# gevrek's boot splash (resources/plymouth/README.md). Installed, not made
# the one in use: `plymouth-set-default-theme gevrek -R` does that.
install -d %{buildroot}%{_datadir}/plymouth/themes/gevrek
install -m 0644 resources/plymouth/gevrek/gevrek.plymouth resources/plymouth/gevrek/*.png %{buildroot}%{_datadir}/plymouth/themes/gevrek/

%post
# Where SELinux is on. A machine without it is as it was.
if command -v semodule >/dev/null 2>&1 && command -v selinuxenabled >/dev/null 2>&1 && selinuxenabled; then
    semodule -i %{_datadir}/selinux/packages/gevrek-dm.cil || :
    restorecon -R %{_bindir}/gevrek-dm %{_bindir}/gevrek-lock %{_sysconfdir}/pam.d/gevrek-dm \n        %{_sysconfdir}/pam.d/gevrek-greeter %{_unitdir}/gevrek-dm.service 2>/dev/null || :
fi

%postun
# Taken away for good (not an update): the module goes with it.
if [ "$1" -eq 0 ] && command -v semodule >/dev/null 2>&1; then
    semodule -r gevrek-dm >/dev/null 2>&1 || :
fi

%files
%license LICENSE
%doc docs/LINUX.md
%{_bindir}/gevrek
%{_bindir}/gevrek-wl
%{_bindir}/gevrek-studio
%{_bindir}/gevrek-shell
%{_bindir}/gevrek-lock
%{_bindir}/gevrek-session
%{_userunitdir}/gevrek.service
%{_userunitdir}/gevrek-safe.service
%{_userunitdir}/gevrek-shell.service
%{_userunitdir}/gevrek-shutdown.target
%{_datadir}/wayland-sessions/gevrek.desktop
%{_datadir}/applications/gevrek-studio.desktop
%{_datadir}/icons/hicolor/*/apps/gevrek.png
%{_datadir}/icons/hicolor/scalable/apps/gevrek.svg
%{_datadir}/xdg-desktop-portal/gevrek-portals.conf
%{_datadir}/xdg-desktop-portal/portals/gevrek.portal
%config(noreplace) %{_sysconfdir}/pam.d/gevrek-lock
%{_bindir}/gevrek-dm
%{_unitdir}/gevrek-dm.service
%{_sysusersdir}/gevrek-dm.conf
%{_tmpfilesdir}/gevrek-dm.conf
%{_datadir}/selinux/packages/gevrek-dm.cil
%config(noreplace) %{_sysconfdir}/pam.d/gevrek-dm
%dir %{_datadir}/plymouth
%dir %{_datadir}/plymouth/themes
%{_datadir}/plymouth/themes/gevrek
%config(noreplace) %{_sysconfdir}/pam.d/gevrek-greeter
%doc resources/dm/README.md
