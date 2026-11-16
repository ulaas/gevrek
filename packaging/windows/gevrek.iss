; gevrek's setup program for Windows (Inno Setup 6). `cargo xtask dist`
; compiles it, with what it needs to know given on the command line:
;
;   ISCC /DVersion=0.6.0 /DNumbers=0.6.0 /DSource=<the unpacked package>
;        /DOut=<folder> /DName=gevrek-windows-x64-setup gevrek.iss
;
; It installs for the person who runs it, with no administrator, to the
; place install.ps1 uses (%LOCALAPPDATA%\Programs\gevrek), and does what
; that script does: the folder on the PATH, the Start menu's entries, a
; gevrek that is running stopped for the update and started again. An
; install made by install.ps1 is taken over.
;
; The uninstaller it leaves (unins000.exe) is also how a running gevrek
; knows that it was installed from the package and may update itself
; (crates/update).

#ifndef Version
  #error Version is not given (cargo xtask dist gives it)
#endif

[Setup]
; Never to be changed: it is how a newer setup finds the older install.
AppId=gevrek
AppName=gevrek
AppVersion={#Version}
AppVerName=gevrek {#Version}
AppPublisher=gevrek
AppPublisherURL=https://ulaas.github.io/gevrek/
AppSupportURL=https://github.com/ulaas/gevrek/issues
AppUpdatesURL=https://github.com/ulaas/gevrek/releases
VersionInfoVersion={#Numbers}
DefaultDirName={localappdata}\Programs\gevrek
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Out}
OutputBaseFilename={#Name}
SetupIconFile=..\..\assets\brand\app-icon\windows\gevrek.ico
UninstallDisplayIcon={app}\gevrek-studio.exe
UninstallDisplayName=gevrek
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The PATH changes: programs that are open are told.
ChangesEnvironment=yes
; gevrek is stopped by its own command (below), which puts every window
; back; it is not a program to be closed from outside.
CloseApplications=no

[Tasks]
Name: "autostart"; Description: "Start gevrek when I sign in"; Flags: unchecked

[Files]
Source: "{#Source}\gevrek.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\gevrek-wm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\gevrek-studio.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\gevrek-shell.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; Where install.ps1 puts them, by the same names.
Name: "{userprograms}\gevrek\gevrek settings"; Filename: "{app}\gevrek-studio.exe"; WorkingDir: "{app}"; Comment: "gevrek's settings program"
Name: "{userprograms}\gevrek\gevrek (start)"; Filename: "{app}\gevrek.exe"; Parameters: "start all"; WorkingDir: "{app}"; Comment: "Start the window manager and the shell"
Name: "{userprograms}\gevrek\gevrek (stop)"; Filename: "{app}\gevrek.exe"; Parameters: "stop all"; WorkingDir: "{app}"; Comment: "Stop gevrek; every window is put back"

[Run]
Filename: "{app}\gevrek.exe"; Parameters: "enable-autostart"; Tasks: autostart; Flags: runhidden
; A gevrek that was running is started again whatever is asked (below);
; one that was not, if the person says so.
Filename: "{app}\gevrek.exe"; Parameters: "start all"; Description: "Start gevrek now"; Flags: postinstall nowait skipifsilent runhidden; Check: not WasRunning

[UninstallRun]
; Not started at sign-in any more, and not running: every window gevrek
; hid is put back by its stop.
Filename: "{app}\gevrek.exe"; Parameters: "disable-autostart"; Flags: runhidden; RunOnceId: "NoAutostart"
Filename: "{app}\gevrek.exe"; Parameters: "stop all"; Flags: runhidden; RunOnceId: "Stop"

[UninstallDelete]
; The build before, which an update keeps, and what install.ps1 left.
Type: filesandordirs; Name: "{app}\previous"
Type: files; Name: "{app}\uninstall.ps1"

[Code]
const
  Environment = 'Environment';
  OldEntry = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\gevrek';

var
  Running: Boolean;

function WasRunning: Boolean;
begin
  Result := Running;
end;

{ Whether the installed gevrek says that its window manager runs. }
function GevrekRuns(Program_: String): Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{cmd}'),
    '/C ""' + Program_ + '" status | "' + ExpandConstant('{sys}') + '\findstr.exe" /R /C:"wm *: *running""',
    '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

{ Before a file is written: a gevrek that runs is stopped, and remembered. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Program_: String;
  Code: Integer;
begin
  Result := '';
  Running := False;
  Program_ := ExpandConstant('{app}\gevrek.exe');
  if FileExists(Program_) then
  begin
    Running := GevrekRuns(Program_);
    { Also when the window manager does not run: the settings program may. }
    Exec(Program_, 'stop all', '', SW_HIDE, ewWaitUntilTerminated, Code);
    if Running then
      Sleep(2000);
  end;
end;

{ The person's PATH, as it is written: with its %NAMES% not filled in. }
procedure AddToPath(Folder: String);
var
  Paths: String;
begin
  if not RegQueryStringValue(HKCU, Environment, 'Path', Paths) then
    Paths := '';
  if Pos(';' + Uppercase(Folder) + ';', ';' + Uppercase(Paths) + ';') > 0 then
    exit;
  if (Paths <> '') and (Paths[Length(Paths)] <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(HKCU, Environment, 'Path', Paths + Folder);
end;

procedure RemoveFromPath(Folder: String);
var
  Paths: String;
  At: Integer;
begin
  if not RegQueryStringValue(HKCU, Environment, 'Path', Paths) then
    exit;
  At := Pos(';' + Uppercase(Folder) + ';', ';' + Uppercase(Paths) + ';');
  if At = 0 then
    exit;
  { The folder, and the mark after it. }
  Delete(Paths, At, Length(Folder) + 1);
  if (Paths <> '') and (Paths[Length(Paths)] = ';') then
    Delete(Paths, Length(Paths), 1);
  RegWriteExpandStringValue(HKCU, Environment, 'Path', Paths);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    AddToPath(ExpandConstant('{app}'));
    { An install made by install.ps1 is this setup's now: its note of how
      to remove it goes, and its script (the uninstaller takes its place). }
    RegDeleteKeyIncludingSubkeys(HKCU, OldEntry);
    DeleteFile(ExpandConstant('{app}\uninstall.ps1'));
    if Running then
      Exec(ExpandConstant('{app}\gevrek.exe'), 'start all', '', SW_HIDE, ewNoWait, Code);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
