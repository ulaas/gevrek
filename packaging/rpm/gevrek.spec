# gevrek as an RPM, from programs that are built already
# (`cargo xtask rpm` builds them and calls rpmbuild with this file).
# A package built from source for a repository comes later; this one is
# for installing the build of this tree on the machine it was built on,
# or one like it.

Name:           gevrek
Version:        %{gevrek_version}
Release:        %{gevrek_release}%{?dist}
Summary:        A tiling window manager and desktop for Wayland
License:        MIT
URL:            https://github.com/ulaas/gevrek

# The programs are built by cargo before rpmbuild runs.
%global debug_package %{nil}
%global _build_id_links none
AutoReqProv:    yes

Requires:       polkit
Requires:       mesa-dri-drivers
Recommends:     xdg-desktop-portal
Recommends:     xdg-desktop-portal-gtk
Recommends:     xorg-x11-server-Xwayland
Recommends:     pipewire-utils
Recommends:     foot

%description
gevrek is a tiling Wayland compositor with its own desktop around it: a
panel, a launcher, notifications, a lock screen, an authentication agent,
and a settings program (gevrek-studio) in which every setting has a form.
After installing, gevrek is in the login manager's list of sessions.

%install
install -d %{buildroot}%{_bindir} %{buildroot}%{_userunitdir}
for program in gevrek gevrek-wl gevrek-studio gevrek-shell gevrek-lock; do
    install -m 0755 %{gevrek_programs}/$program %{buildroot}%{_bindir}/$program
done
install -m 0755 %{gevrek_resources}/gevrek-session %{buildroot}%{_bindir}/gevrek-session
# The units name the programs by their place.
for unit in gevrek.service gevrek-safe.service gevrek-shell.service gevrek-shutdown.target; do
    sed 's|@BIN@|%{_bindir}|g' %{gevrek_resources}/systemd/$unit > %{buildroot}%{_userunitdir}/$unit
    chmod 0644 %{buildroot}%{_userunitdir}/$unit
done
install -D -m 0644 %{gevrek_resources}/gevrek.desktop %{buildroot}%{_datadir}/wayland-sessions/gevrek.desktop
install -D -m 0644 %{gevrek_resources}/gevrek-studio.desktop %{buildroot}%{_datadir}/applications/gevrek-studio.desktop
# gevrek's icon, by the name the application's entry gives (`Icon=gevrek`).
for size in 16 24 32 48 64 128 256 512; do
    install -D -m 0644 %{gevrek_root}/assets/brand/app-icon/linux/hicolor/${size}x${size}/apps/gevrek.png %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/gevrek.png
done
install -D -m 0644 %{gevrek_root}/assets/brand/app-icon/linux/hicolor/scalable/apps/gevrek.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/gevrek.svg
install -D -m 0644 %{gevrek_resources}/gevrek-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/gevrek-portals.conf
install -D -m 0644 %{gevrek_resources}/gevrek.portal %{buildroot}%{_datadir}/xdg-desktop-portal/portals/gevrek.portal
# The lock screen checks the password as every login on the system does.
install -d %{buildroot}%{_sysconfdir}/pam.d
cat > %{buildroot}%{_sysconfdir}/pam.d/gevrek-lock <<'PAM'
#%PAM-1.0
auth       include      system-auth
account    include      system-auth
PAM
install -D -m 0644 %{gevrek_root}/LICENSE %{buildroot}%{_licensedir}/gevrek/LICENSE
install -D -m 0644 %{gevrek_root}/docs/LINUX.md %{buildroot}%{_docdir}/gevrek/LINUX.md

%files
%license %{_licensedir}/gevrek/LICENSE
%doc %{_docdir}/gevrek/LINUX.md
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
