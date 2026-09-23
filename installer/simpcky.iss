; Instalador de Simpcky (Inno Setup 6).
;
; Compilar (lo hace GitHub Actions en cada versión, ver
; .github/workflows/release.yml):
;   ISCC.exe /DAppVersion=0.3.0 installer\simpcky.iss
; Sale en dist\Simpcky-Setup-<versión>.exe.
;
; - Para el usuario actual, sin pedir permisos de administrador
;   (%LOCALAPPDATA%\Programs\Simpcky).
; - Antes de reemplazar el .exe, cierra la app abierta de forma
;   ordenada (simpcky.exe --quit: guarda lo que se esté escribiendo).
; - El actualizador de la app lo ejecuta en silencio con /RELAUNCH:
;   ahí, al terminar, vuelve a abrir Simpcky.
; - Al desinstalar: limpia el registro (inicio con Windows, clic derecho
;   del escritorio) y pregunta si borrar también las notas.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#define AppName "Simpcky"
#define AppExe "simpcky.exe"
#define Repo "https://github.com/santiagoPostacchini/Simpcky"
#define Site "https://santiagopostacchini.github.io/Simpcky/"

[Setup]
; Identidad fija del programa: nunca cambiarla (con ella Windows sabe
; que una versión nueva actualiza a la anterior).
AppId={{D6720743-4439-4DE5-9E06-D6E7A8DEDB86}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Santiago Postacchini
AppPublisherURL={#Site}
AppSupportURL={#Repo}/issues
AppUpdatesURL={#Repo}/releases
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Instalador de {#AppName}
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableDirPage=auto
DisableProgramGroupPage=yes
DisableReadyPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1903 en adelante (los menús oscuros y el resto del sistema
; que usa la app); pensada para Windows 11.
MinVersion=10.0.18362
OutputDir=..\dist
OutputBaseFilename=Simpcky-Setup-{#AppVersion}
SetupIconFile=..\assets\simpcky.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Si igual quedara abierta, que el propio instalador la cierre (Restart
; Manager: la app responde a ENDSESSION_CLOSEAPP cerrándose).
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "autostart"; Description: "Iniciar {#AppName} con Windows (recomendado: las notas viven en el escritorio)"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Notas adhesivas para el escritorio"

[Registry]
; Iniciar con Windows (el mismo valor que escribe la app desde su menú).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart; Flags: uninsdeletevalue
; Al desinstalar, borrar lo que la app haya registrado por su cuenta,
; aunque no lo haya creado el instalador.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "{#AppName}"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\DesktopBackground\Shell\{#AppName}"; Flags: uninsdeletekey dontcreatekey

[Run]
; Instalación normal: casilla "Abrir Simpcky" al final.
Filename: "{app}\{#AppExe}"; Description: "Abrir {#AppName}"; Flags: nowait postinstall skipifsilent
; Actualización desde la app (silenciosa, con /RELAUNCH): volver a abrirla.
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: IsRelaunch

[UninstallRun]
Filename: "{app}\{#AppExe}"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "CerrarSimpcky"

[Code]
function IsRelaunch: Boolean;
begin
  Result := WizardSilent and (Pos('/RELAUNCH', UpperCase(GetCmdTail)) > 0);
end;

// Antes de copiar el .exe nuevo: cerrar la versión abierta guardando
// todo (simpcky.exe --quit espera a que termine de cerrarse).
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\{#AppExe}');
  if FileExists(Exe) then
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

// Al terminar de desinstalar: las notas no se borran solas. Se pregunta
// (salvo en una desinstalación silenciosa), con "No" por defecto.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
  begin
    Data := ExpandConstant('{userappdata}\{#AppName}');
    if DirExists(Data) then
      if MsgBox('¿Borrar también tus notas y ajustes de Simpcky?' + #13#10#13#10 +
                'Si sincronizabas con Google Drive, la copia de Drive no se toca.' + #13#10 +
                'Si decís que no, quedan en ' + Data + ' por si volvés a instalarlo.',
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(Data, True, True, True);
  end;
end;
