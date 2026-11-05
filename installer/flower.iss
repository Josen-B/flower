#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef BuildId
  #error BuildId is required
#endif
#ifndef FileVersion
  #error FileVersion is required
#endif
#ifndef PackageDir
  #error PackageDir is required
#endif
#ifndef FilesInclude
  #error FilesInclude is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef ExpectedDllHash
  #error ExpectedDllHash is required
#endif
#ifndef ExpectedToolHash
  #error ExpectedToolHash is required
#endif
#ifndef EngineLicense
  #error EngineLicense is required
#endif

[Setup]
AppId={{1B8DAF2E-D5F7-49AC-A9BF-4713A0D57224}
AppName=Flower
AppVersion={#AppVersion}
VersionInfoVersion={#FileVersion}
VersionInfoTextVersion={#AppVersion}
VersionInfoProductName=Flower
VersionInfoDescription=Flower 测试版安装器
DefaultDirName={commonpf64}\Flower
DefaultGroupName=Flower
DisableDirPage=yes
UninstallFilesDir={commonpf64}\Flower
AllowUNCPath=no
DisableProgramGroupPage=yes
UsePreviousAppDir=no
PrivilegesRequired=admin
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=flower-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\flower.ico
UninstallDisplayIcon={commonpf64}\Flower\versions\{#BuildId}\flower-settings.exe
LicenseFile={#PackageDir}\LICENSE-MIT
InfoBeforeFile=notice.zh-CN.txt
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableWelcomePage=no
CloseApplications=no
RestartApplications=no
AllowCancelDuringInstall=no
RestartIfNeededByRun=no
SetupLogging=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "zh"; MessagesFile: "compiler:Default.isl,zh-CN.isl"

[Files]
Source: "{#PackageDir}\flower-register.exe"; DestName: "flower-path-check.exe"; Flags: dontcopy
#include FilesInclude
Source: "notice.zh-CN.txt"; DestDir: "{commonpf64}\Flower\versions\{#BuildId}"; DestName: "INSTALLER-README.txt"; Flags: ignoreversion
Source: "{#EngineLicense}"; DestDir: "{commonpf64}\Flower\versions\{#BuildId}\licenses\installer"; DestName: "INNO-SETUP.txt"; Flags: ignoreversion

[Icons]
Name: "{commonprograms}\Flower\启用 Flower（当前用户）"; Filename: "{commonpf64}\Flower\versions\{#BuildId}\flower-register.exe"; Parameters: "enable"; IconFilename: "{commonpf64}\Flower\versions\{#BuildId}\flower-settings.exe"
Name: "{commonprograms}\Flower\停用 Flower（当前用户）"; Filename: "{commonpf64}\Flower\versions\{#BuildId}\flower-register.exe"; Parameters: "disable"; IconFilename: "{commonpf64}\Flower\versions\{#BuildId}\flower-settings.exe"
Name: "{commonprograms}\Flower\Flower 设置"; Filename: "{commonpf64}\Flower\versions\{#BuildId}\flower-settings.exe"
Name: "{commonprograms}\Flower\Flower 安装说明"; Filename: "{commonpf64}\Flower\versions\{#BuildId}\INSTALLER-README.txt"

[Code]
const
  FlowerClass = '{3D5B7CB1-96CB-4F44-A792-64304F6E7426}';
  ClassKey = 'Software\Classes\CLSID\{3D5B7CB1-96CB-4F44-A792-64304F6E7426}';
  TipKey = 'Software\Microsoft\CTF\TIP\{3D5B7CB1-96CB-4F44-A792-64304F6E7426}';
  KeyboardCategory = '{34745C63-B2F0-4784-8B67-5E12C8701A31}';
  InstallerKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{1B8DAF2E-D5F7-49AC-A9BF-4713A0D57224}_is1';
var
  RegistrationFailed: Boolean;
  PathCheckerReady: Boolean;
  UserFilesFailed: Boolean;
  UserFilesPage: TInputDirWizardPage;

function SystemRoot: String;
begin
  Result := ExpandConstant('{commonpf64}\Flower');
end;

procedure InitializeWizard;
begin
  UserFilesPage := CreateInputDirPage(wpSelectDir, '用户文件目录',
    '系统组件与用户副本分开部署',
    '机器 DLL、注册工具和卸载器固定安装到 ' + SystemRoot +
    '。下方仅存放设置程序、说明和许可副本，不是 DLL 安装位置。' +
    '默认值来自安装进程的 LOCALAPPDATA；若使用另一管理员账户提升，请改选原始用户目录。',
    False, '');
  UserFilesPage.Add('用户文件副本目录：');
  UserFilesPage.Values[0] := GetEnv('LOCALAPPDATA') + '\Flower\UserFiles';
end;

function CreateFileW(Name: String; Access, Share: LongWord; Security: LongWord;
  Creation, Attributes: LongWord; Template: THandle): THandle;
  external 'CreateFileW@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';
function GetFinalPathNameByHandleW(Handle: THandle; Buffer: String;
  BufferLength, Flags: LongWord): LongWord;
  external 'GetFinalPathNameByHandleW@kernel32.dll stdcall';
function CompareStringOrdinal(Left: String; LeftLength: Integer;
  Right: String; RightLength: Integer; IgnoreCase: Boolean): Integer;
  external 'CompareStringOrdinal@kernel32.dll stdcall';

function PayloadDir: String;
begin
  Result := ExpandConstant('{commonpf64}\Flower\versions\{#BuildId}');
end;

function PathChecker: String;
begin
  Result := ExpandConstant('{tmp}\flower-path-check.exe');
end;

function SafeInstallPath(Name: String): Boolean;
var
  ExitCode: Integer;
begin
  Result := False;
  if not PathCheckerReady then Exit;
  if CompareText(GetSHA256OfFile(PathChecker), '{#ExpectedToolHash}') <> 0 then Exit;
  if Exec(PathChecker, 'check-install-path --path "' + Name + '"',
    ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Result := ExitCode = 0;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  ExitCode: Integer;
begin
  Result := True;
  if CurPageID <> UserFilesPage.ID then Exit;
  if not PathCheckerReady then begin
    ExtractTemporaryFile('flower-path-check.exe');
    PathCheckerReady := True;
  end;
  Result := False;
  if CompareText(GetSHA256OfFile(PathChecker), '{#ExpectedToolHash}') <> 0 then Exit;
  if Exec(PathChecker, 'check-user-install-path --path "' + UserFilesPage.Values[0] + '"',
    ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Result := ExitCode = 0;
  if not Result then
    MsgBox('用户文件目录无效、含链接或固定目标文件已存在。请选择本地固定盘正常目录，例如 D:\software\Flower。不会更改已有权限；普通用户写入权限在部署时检查。', mbError, MB_OK);
end;

function PrepareInstallPath(Name: String): Boolean;
var
  ExitCode: Integer;
begin
  Result := False;
  if not PathCheckerReady then Exit;
  if CompareText(GetSHA256OfFile(PathChecker), '{#ExpectedToolHash}') <> 0 then Exit;
  if Exec(PathChecker, 'prepare-install-path --path "' + Name + '"',
    ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Result := ExitCode = 0;
end;

function HasSubkeys(Key: String): Boolean;
var
  Names: TArrayOfString;
begin
  Result := False;
  if not RegKeyExists(HKLM64, Key) then Exit;
  if not RegGetSubkeyNames(HKLM64, Key, Names) then begin
    Result := True;
    Exit;
  end;
  Result := GetArrayLength(Names) > 0;
end;

function HasFlowerRegistration: Boolean;
var
  Languages: TArrayOfString;
  I: Integer;
begin
  Result := True;
  if RegKeyExists(HKLM64, ClassKey) then Exit;
  if HasSubkeys(TipKey + '\Category\Item\' + FlowerClass) then Exit;
  if RegKeyExists(HKLM64, TipKey + '\Category\Category\' +
    KeyboardCategory + '\' + FlowerClass) then Exit;
  if RegKeyExists(HKLM64, TipKey + '\LanguageProfile') then begin
    if not RegGetSubkeyNames(HKLM64, TipKey + '\LanguageProfile', Languages) then Exit;
    for I := 0 to GetArrayLength(Languages) - 1 do
      if HasSubkeys(TipKey + '\LanguageProfile\' + Languages[I]) then Exit;
  end;
  Result := False;
end;

function CanonicalFile(Name: String): String;
var
  Handle: THandle;
  Buffer: String;
  Count: LongWord;
begin
  Result := '';
  Handle := CreateFileW(Name, $80000000, 7, 0, 3, 0, 0);
  if Handle = THandle(-1) then Exit;
  try
    SetLength(Buffer, 32768);
    Count := GetFinalPathNameByHandleW(Handle, Buffer, Length(Buffer), 0);
    if (Count > 0) and (Count < LongWord(Length(Buffer))) then begin
      SetLength(Buffer, Count);
      Result := Buffer;
    end;
  finally
    CloseHandle(Handle);
  end;
end;

function DllNotInUse: Boolean;
var
  Name: String;
  Handle: THandle;
begin
  Name := PayloadDir + '\flower.dll';
  Result := True;
  if not FileExists(Name) then Exit;
  { Request write access exclusively, but never write or truncate the file. }
  Handle := CreateFileW(Name, $C0000000, 0, 0, 3, 0, 0);
  Result := Handle <> THandle(-1);
  if Result then CloseHandle(Handle);
end;

function TrustedPayload: Boolean;
begin
  Result := False;
  if not FileExists(PayloadDir + '\flower.dll') then Exit;
  if not FileExists(PayloadDir + '\flower-register.exe') then Exit;
  Result :=
    (CompareText(GetSHA256OfFile(PayloadDir + '\flower.dll'), '{#ExpectedDllHash}') = 0) and
    (CompareText(GetSHA256OfFile(PayloadDir + '\flower-register.exe'), '{#ExpectedToolHash}') = 0);
end;

function OwnedRegistrationComplete: Boolean;
var
  Registered, Expected, Description: String;
begin
  Result := False;
  if not RegQueryStringValue(HKLM64, ClassKey + '\InprocServer32', '', Registered) then Exit;
  Expected := CanonicalFile(PayloadDir + '\flower.dll');
  if (Expected = '') or (CompareStringOrdinal(Registered, -1, Expected, -1, True) <> 2) then Exit;
  if not RegQueryStringValue(HKLM64, TipKey +
    '\LanguageProfile\0x00000804\{6C7782F4-5F59-42CF-8C29-F9A8D46F1B72}',
    'Description', Description) then Exit;
  Result := (Description = 'Flower') and RegKeyExists(HKLM64,
    TipKey + '\Category\Category\' + KeyboardCategory + '\' + FlowerClass);
end;

function InitializeSetup: Boolean;
begin
  Result := not WizardSilent;
  if not Result then
    SuppressibleMsgBox('此测试版安装器仅支持交互安装。', mbError, MB_OK, IDOK);
end;

function HasNativeRuntime: Boolean;
var
  Previous: Boolean;
begin
  Previous := EnableFsRedirection(False);
  try
    Result := FileExists(ExpandConstant('{sys}\vcruntime140.dll'));
  finally
    EnableFsRedirection(Previous);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if CompareText(WizardDirValue, SystemRoot) <> 0 then
    Result := '系统组件位置固定为 Program Files\Flower，不接受 /DIR 覆盖；请重新运行安装器。'
  else if HasFlowerRegistration or RegKeyExists(HKLM64, InstallerKey) then
    Result := '已发现 Flower 注册或安装记录。请先由各用户停用 Flower、退出旧宿主，再使用旧安装工具注销或卸载；本安装器不会覆盖旧安装。'
  else if DirExists(PayloadDir) then
    Result := '目标版本目录已存在。为保护未知或未完成的安装，拒绝覆盖。'
  else if not HasNativeRuntime then
    Result := '请先安装 Microsoft Visual C++ v14 x64 运行库。本离线安装器不会下载它。'
  else if not PrepareInstallPath(ExpandConstant('{commonpf64}\Flower')) then
    Result := '安全准备目录失败：祖先或已有目标不安全，或创建时发生竞争。已有权限未更改；本次创建的空安全目录可能保留。'
  else if not SafeInstallPath(ExpandConstant('{commonpf64}\Flower')) then
    Result := '系统组件目录安全检查失败。已有目录权限不会更改，请由管理员检查 Program Files\Flower。';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExitCode: Integer;
begin
  if CurStep <> ssPostInstall then Exit;
  RegistrationFailed := True;
  if SafeInstallPath(ExpandConstant('{commonpf64}\Flower')) and TrustedPayload then begin
    if Exec(PayloadDir + '\flower-register.exe',
      'register --dll "' + PayloadDir + '\flower.dll"', PayloadDir,
      SW_HIDE, ewWaitUntilTerminated, ExitCode) then begin
      Log('Flower register exit code: ' + IntToStr(ExitCode));
      RegistrationFailed := (ExitCode <> 0) or (not OwnedRegistrationComplete);
    end;
  end;
  if not RegistrationFailed then begin
    UserFilesFailed := True;
    if SafeInstallPath(SystemRoot) and TrustedPayload then begin
      if ExecAsOriginalUser(PayloadDir + '\flower-register.exe',
        'install-user-files --path "' + UserFilesPage.Values[0] + '"', PayloadDir,
        SW_HIDE, ewWaitUntilTerminated, ExitCode) then begin
        Log('Flower user files exit code: ' + IntToStr(ExitCode));
        UserFilesFailed := ExitCode <> 0;
      end;
    end;
  end;
  if RegistrationFailed then begin
    Log('Flower registration failed; keeping installed files for repair.');
    WizardForm.FinishedLabel.Caption :=
      'Flower 注册失败。文件和卸载入口已保留，不能将本次安装视为成功。请查看安装日志并修复或卸载后重试。';
    SuppressibleMsgBox(WizardForm.FinishedLabel.Caption, mbError, MB_OK, IDOK);
  end else if UserFilesFailed then begin
    WizardForm.FinishedLabel.Caption :=
      '用户文件部署失败，本次安装未完整成功。未回退管理员写入；系统组件及卸载入口已保留，请检查普通用户目录权限、已有文件和日志后卸载重试。';
    SuppressibleMsgBox(WizardForm.FinishedLabel.Caption, mbError, MB_OK, IDOK);
  end else
    WizardForm.FinishedLabel.Caption :=
      'Flower 机器级注册已完成。每个用户仍需从开始菜单运行“启用 Flower（当前用户）”，再按 Win+Space 选择。本测试版尚未通过实际输入验收。';
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and UserFilesFailed then
    WizardForm.FinishedLabel.Caption :=
      '用户文件部署失败，本次安装未完整成功。系统组件和卸载入口已保留，未使用管理员权限写入自选目录。请查看日志并卸载后重试。';
  if (CurPageID = wpFinished) and RegistrationFailed then
    WizardForm.FinishedLabel.Caption :=
      'Flower 注册失败。文件和卸载入口已保留，不能将本次安装视为成功。请查看安装日志并修复或卸载后重试。';
end;

function GetCustomSetupExitCode: Integer;
begin
  Result := 0;
  if RegistrationFailed then Result := 15;
  if UserFilesFailed then Result := 16;
end;

function InitializeUninstall: Boolean;
var
  Registered, Expected: String;
  ExitCode: Integer;
begin
  Result := False;
  if UninstallSilent then begin
    SuppressibleMsgBox('此测试版仅支持交互卸载。', mbError, MB_OK, IDOK);
    Exit;
  end;
  if SuppressibleMsgBox(
    '请确认每个用户已停用 Flower，并保存工作、退出所有使用过 Flower 的应用。卸载不会替其他账户停用；自选目录副本、词典和用户设置均保留，可由用户自行删除。是否继续？',
    mbConfirmation, MB_YESNO, IDNO) <> IDYES then Exit;
  { Never execute a helper directly out of a tree whose ACL may have changed.
    Copy to Inno's private temporary directory, verify the copy, then inspect. }
  if not FileCopy(PayloadDir + '\flower-register.exe', PathChecker, False) then begin
    MsgBox('无法准备只读目录检查工具，已停止卸载并保留文件。', mbError, MB_OK);
    Exit;
  end;
  PathCheckerReady := True;
  if not SafeInstallPath(ExpandConstant('{commonpf64}\Flower')) then begin
    MsgBox('安装目录权限已改变或文件校验失败，已停止卸载并保留文件。请先由管理员检查目录安全性。', mbError, MB_OK);
    Exit;
  end;
  if not DllNotInUse then begin
    MsgBox('Flower DLL 仍被占用或不可写。请退出宿主或注销登录后重试；未开始清理文件。', mbError, MB_OK);
    Exit;
  end;
  if not HasFlowerRegistration then begin
    Result := True;
    Exit;
  end;
  if not RegQueryStringValue(HKLM64, ClassKey + '\InprocServer32', '', Registered) then begin
    MsgBox('Flower 注册残缺，无法确认路径所有权。已停止卸载并保留文件。', mbError, MB_OK);
    Exit;
  end;
  Expected := CanonicalFile(PayloadDir + '\flower.dll');
  if (Expected = '') or
     (CompareStringOrdinal(Registered, -1, Expected, -1, True) <> 2) or
     (not TrustedPayload) then begin
    MsgBox('当前 Flower 注册不属于本次文件，或文件校验失败。已停止卸载，未触及其他安装。', mbError, MB_OK);
    Exit;
  end;
  if not Exec(PayloadDir + '\flower-register.exe', 'unregister', PayloadDir,
    SW_HIDE, ewWaitUntilTerminated, ExitCode) then begin
    MsgBox('无法启动 Flower 注册清理工具，已停止卸载并保留文件。', mbError, MB_OK);
    Exit;
  end;
  if (ExitCode <> 0) or HasFlowerRegistration or (not DllNotInUse) then begin
    MsgBox('Flower 注册清理或占用复查失败。已停止文件清理，保留 DLL 和工具供修复后重试。', mbError, MB_OK);
    Exit;
  end;
  Result := True;
end;
