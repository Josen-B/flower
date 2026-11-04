//! Read-only, fail-closed checks for machine-wide executable installation paths.
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetAce, IsValidSid, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetFileInformationByHandle, QueryDosDeviceW, BY_HANDLE_FILE_INFORMATION,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;

const SYSTEM: &str = "S-1-5-18";
const ADMINS: &str = "S-1-5-32-544";
const INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
const CREATOR_OWNER: &str = "S-1-3-0";
const INHERIT_ONLY: u8 = 0x08;
const REPARSE_POINT: u32 = 0x400;
// DELETE, WRITE_DAC, WRITE_OWNER, generic write/all, MAXIMUM_ALLOWED,
// WRITE_EA, WRITE_ATTRIBUTES and FILE_DELETE_CHILD.
const ANCESTOR_MUTATION: u32 = 0x520d_0150;
const TREE_MUTATION: u32 = ANCESTOR_MUTATION | 0x6; // write/add file, append/add subdirectory

fn trusted(sid: &str) -> bool {
    matches!(sid, SYSTEM | ADMINS | INSTALLER)
}

fn allow_is_safe(sid: &str, mask: u32, flags: u8, strict: bool) -> bool {
    if !strict && flags & INHERIT_ONLY != 0 {
        return true;
    }
    if trusted(sid) || (sid == CREATOR_OWNER && flags & INHERIT_ONLY != 0) {
        return true;
    }
    mask & if strict {
        TREE_MUTATION
    } else {
        ANCESTOR_MUTATION
    } == 0
}

fn validate_spelling(text: &str) -> Result<(), String> {
    let b = text.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || &b[1..3] != b":\\" {
        return Err("只接受本地盘符绝对目录（例如 D:\\Software\\Flower），不能为卷根。".into());
    }
    if text.len() > 220 {
        return Err("安装根目录过长（最多 220 UTF-8 字节）。".into());
    }
    for part in text[3..].split('\\') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || stem.strip_prefix("COM").is_some_and(reserved_number)
            || stem.strip_prefix("LPT").is_some_and(reserved_number);
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part.chars().any(|c| c < ' ' || r#"/:*?"<>|"#.contains(c))
            || reserved
        {
            return Err("目录含空段、保留名称、点路径或 Windows 不允许的字符。".into());
        }
    }
    Ok(())
}

fn reserved_number(s: &str) -> bool {
    matches!(
        s,
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
    )
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

struct LocalAllocation(*mut std::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(HLOCAL(self.0));
        }
    }
}

unsafe fn sid_string(sid: PSID) -> Result<String, String> {
    if sid.0.is_null() || !IsValidSid(sid).as_bool() {
        return Err("无效 SID".into());
    }
    let mut result = PWSTR::null();
    ConvertSidToStringSidW(sid, &mut result).map_err(|e| e.to_string())?;
    let _memory = LocalAllocation(result.0.cast());
    result.to_string().map_err(|e| e.to_string())
}

fn inspect(path: &Path, strict: bool) -> Result<bool, String> {
    // Do not follow the final component; every ancestor is independently inspected.
    // Keep a no-delete-share handle open throughout the descriptor inspection.
    let file = OpenOptions::new()
        .access_mode(0x0002_0000) // READ_CONTROL
        .share_mode(3)
        .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("不允许重解析点、符号链接或目录联接".into());
    }
    unsafe {
        let handle = HANDLE(file.as_raw_handle());
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(handle, &mut info).map_err(|e| e.to_string())?;
        if !metadata.is_dir() && info.nNumberOfLinks != 1 {
            return Err("不允许多硬链接文件".into());
        }
        let mut owner = PSID::default();
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let status = GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            Some(&mut descriptor),
        );
        let _memory = LocalAllocation(descriptor.0);
        status.ok().map_err(|e| e.to_string())?;
        inspect_acl(owner, acl, strict)?;
    }
    Ok(metadata.is_dir())
}

// The pointers must belong to a live, OS-validated security descriptor.
unsafe fn inspect_acl(owner: PSID, acl: *const ACL, strict: bool) -> Result<(), String> {
    if !trusted(&sid_string(owner)?) {
        return Err("owner 必须为 SYSTEM、Administrators 或 TrustedInstaller".into());
    }
    if acl.is_null() {
        return Err("不允许 null DACL".into());
    }
    for index in 0..(*acl).AceCount {
        let mut ace = std::ptr::null_mut();
        GetAce(acl, u32::from(index), &mut ace).map_err(|e| e.to_string())?;
        let header = std::slice::from_raw_parts(ace.cast::<u8>(), 4);
        let size = u16::from_le_bytes([header[2], header[3]]) as usize;
        match header[0] {
            1 => continue, // ACCESS_DENIED_ACE never grants rights.
            0 => {}
            _ => return Err("无法识别的 DACL ACE，保守拒绝".into()),
        }
        if size < 16 {
            return Err("无效 allow ACE".into());
        }
        let bytes = std::slice::from_raw_parts(ace.cast::<u8>(), size);
        let mask = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let sid = sid_string(PSID(ace.cast::<u8>().add(8).cast()))?;
        if !allow_is_safe(&sid, mask, header[1], strict) {
            return Err(format!("非可信 SID {sid} 具有写入或替换权限"));
        }
    }
    Ok(())
}

/// Validate a prospective root or an existing installation tree, without writes.
pub fn check_install_path(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("目录必须为有效 Unicode")?;
    validate_spelling(text)?;
    unsafe {
        let root = wide(OsStr::new(&text[..3]));
        if GetDriveTypeW(PCWSTR(root.as_ptr())) != 3 {
            return Err("只接受本地固定磁盘。".into());
        }
        let drive = wide(OsStr::new(&text[..2]));
        let mut device = [0u16; 1024];
        let count = QueryDosDeviceW(PCWSTR(drive.as_ptr()), Some(&mut device));
        if count == 0 {
            return Err("无法确认磁盘设备。".into());
        }
        let device = String::from_utf16_lossy(&device[..count as usize]);
        if !device.starts_with("\\Device\\HarddiskVolume") {
            return Err("不允许 SUBST、网络映射或未知磁盘设备。".into());
        }
        let mut windows = [0u16; 32768];
        let count = GetWindowsDirectoryW(Some(&mut windows));
        if count == 0 || count as usize >= windows.len() {
            return Err("无法确定 Windows 系统目录。".into());
        }
        let system = String::from_utf16_lossy(&windows[..count as usize]).to_lowercase();
        let candidate = text.to_lowercase();
        if candidate == system || candidate.starts_with(&(system + "\\")) {
            return Err("不能安装到 Windows 系统目录树。".into());
        }
    }
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    let mut existing = Vec::new();
    let mut missing = false;
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(_) if !missing => existing.push(ancestor),
            Ok(_) => return Err("目录结构在检查期间发生变化。".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(e) => return Err(format!("{}：{e}", ancestor.display())),
        }
    }
    for (index, ancestor) in existing.iter().enumerate() {
        let strict = index + 1 == existing.len();
        let directory = inspect(ancestor, strict)
            .map_err(|e| format!("安装目录不安全：{}：{e}", ancestor.display()))?;
        if !directory {
            return Err(format!("{} 不是目录", ancestor.display()));
        }
    }
    if !missing {
        let mut pending = vec![PathBuf::from(path)];
        let mut count = 0usize;
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                count += 1;
                if count > 4096 {
                    return Err("安装树过大，拒绝检查未知目录。".into());
                }
                if inspect(&entry.path(), true)
                    .map_err(|e| format!("{}：{e}", entry.path().display()))?
                {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_sddl(sddl: &str, strict: bool) -> Result<(), String> {
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
        use windows::Win32::Security::{GetSecurityDescriptorDacl, GetSecurityDescriptorOwner};
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let input = wide(OsStr::new(sddl));
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(input.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
            .unwrap();
            let _memory = LocalAllocation(descriptor.0);
            let mut owner = PSID::default();
            let mut defaulted = BOOL::default();
            let mut present = BOOL::default();
            let mut acl = std::ptr::null_mut();
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted).unwrap();
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted).unwrap();
            inspect_acl(owner, acl, strict)
        }
    }

    #[test]
    fn native_descriptors_are_fail_closed_without_changing_filesystem_acl() {
        assert!(check_sddl("O:BAD:(A;;FA;;;BA)(A;;FRFX;;;BU)", true).is_ok());
        assert!(check_sddl("O:SYD:", true).is_ok()); // empty DACL
        assert!(check_sddl("O:BAD:NO_ACCESS_CONTROL", true).is_err());
        assert!(check_sddl("O:BUD:(A;;FA;;;BA)", true).is_err()); // untrusted owner
        assert!(check_sddl("O:BAD:(A;;FW;;;AU)", true).is_err());
        assert!(check_sddl("O:BAD:(D;;FA;;;AU)(A;;FA;;;AU)", true).is_err());
        assert!(check_sddl("O:BAD:(A;OICIIO;FA;;;CO)(A;;FA;;;BA)", true).is_ok());
        assert!(check_sddl("O:BAD:(A;OICIIO;FA;;;AU)", true).is_err());
        assert!(check_sddl("O:BAD:(A;OICIIO;FA;;;AU)", false).is_ok());
        // FILE_DELETE_CHILD (SDDL "DC" is a directory-service right, not this bit).
        assert!(check_sddl("O:BAD:(A;;0x40;;;AU)", false).is_err());
        // Object ACEs are intentionally outside this conservative policy.
        assert!(check_sddl(
            "O:BAD:(OA;;FA;00112233-4455-6677-8899-aabbccddeeff;;BA)",
            true
        )
        .is_err());
    }

    #[test]
    fn accepts_drive_paths_not_unsafe_windows_spellings() {
        for p in [
            r"C:\Program Files\Flower",
            r"D:\Software\Flower",
            r"D:\软件\Flower",
        ] {
            assert!(validate_spelling(p).is_ok(), "{p}");
        }
        for p in [
            r"D:\",
            r"D:Flower",
            r"\\server\share\Flower",
            r"\\?\D:\Flower",
            r"D:\a\..\Flower",
            r"D:\a.\Flower",
            r"D:\a \Flower",
            r"D:\a:ads",
            r"D:\CON.txt",
            r"D:\COM1",
            r"D:\LPT²",
            r"D:\\Flower",
            "D:\\a\0b",
            r"D:/Flower",
        ] {
            assert!(validate_spelling(p).is_err(), "{p}");
        }
    }

    #[test]
    fn acl_policy_rejects_untrusted_writes_and_ancestor_replacement() {
        for mask in [
            2, 4, 0x10, 0x40, 0x100, 0x10000, 0x40000, 0x80000, 0x40000000, 0x10000000,
        ] {
            assert!(!allow_is_safe("S-1-5-11", mask, 0, true));
            assert!(allow_is_safe(ADMINS, mask, 0, true));
        }
        assert!(allow_is_safe("S-1-5-11", 0x1200a9, 0, true)); // read/execute
        assert!(allow_is_safe("S-1-5-11", 4, 0, false)); // root may create new dirs
        assert!(!allow_is_safe("S-1-5-11", 0x40, 0, false)); // not replace children
        assert!(allow_is_safe("S-1-5-11", 0x1f01ff, INHERIT_ONLY, false));
        assert!(!allow_is_safe("S-1-5-11", 0x1f01ff, INHERIT_ONLY, true));
        assert!(allow_is_safe(CREATOR_OWNER, 0x1f01ff, INHERIT_ONLY, true));
        assert!(!allow_is_safe(CREATOR_OWNER, 0x1f01ff, 0, true));
    }
}
