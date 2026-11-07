//! Exact, current-installation ownership of the current user's Run value.
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Value {
    kind: u32,
    bytes: Vec<u8>,
}

trait RegistryStore {
    fn read(&self) -> Result<Option<Value>, String>;
    fn write(&self, value: &Value) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

fn expected(path: &Path) -> Result<Value, String> {
    let path = path.to_str().ok_or("启动路径不是有效 Unicode。")?;
    if !Path::new(path).is_absolute() || path.contains(['"', '\0']) {
        return Err("启动路径必须是无引号的绝对路径。".into());
    }
    Ok(Value {
        kind: 1, // REG_SZ, including exactly one UTF-16 terminator.
        bytes: format!("\"{path}\" --startup-input-method")
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect(),
    })
}

fn update(store: &impl RegistryStore, path: &Path, enable: bool) -> Result<(), String> {
    let expected = expected(path)?;
    match store.read()? {
        Some(value) if value != expected => {
            Err("Flower 自启动项不属于本次安装，已保留原值。".into())
        }
        Some(_) if !enable => store.delete(),
        None if enable => store.write(&expected),
        _ => Ok(()),
    }
}

#[cfg(windows)]
mod registry {
    use super::*;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::*;

    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    struct Store(HKEY);
    impl Drop for Store {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
    fn check(code: windows::Win32::Foundation::WIN32_ERROR) -> Result<(), String> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err("无法访问当前用户自启动项。".into())
        }
    }
    impl RegistryStore for Store {
        fn read(&self) -> Result<Option<Value>, String> {
            let mut kind = REG_VALUE_TYPE::default();
            let mut size = 0;
            unsafe {
                let result = RegQueryValueExW(
                    self.0,
                    w!("Flower"),
                    None,
                    Some(&mut kind),
                    None,
                    Some(&mut size),
                );
                if result == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                check(result)?;
                if size > 65536 {
                    return Err("自启动项过长，已保留原值。".into());
                }
                let mut bytes = vec![0; size as usize];
                check(RegQueryValueExW(
                    self.0,
                    w!("Flower"),
                    None,
                    Some(&mut kind),
                    Some(bytes.as_mut_ptr()),
                    Some(&mut size),
                ))?;
                bytes.truncate(size as usize);
                Ok(Some(Value {
                    kind: kind.0,
                    bytes,
                }))
            }
        }
        fn write(&self, value: &Value) -> Result<(), String> {
            unsafe {
                check(RegSetValueExW(
                    self.0,
                    w!("Flower"),
                    0,
                    REG_VALUE_TYPE(value.kind),
                    Some(&value.bytes),
                ))
            }
        }
        fn delete(&self) -> Result<(), String> {
            unsafe { check(RegDeleteValueW(self.0, w!("Flower"))) }
        }
    }
    pub(super) fn set(path: &Path, enable: bool) -> Result<(), String> {
        let mut key = HKEY::default();
        unsafe {
            let result = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                RUN,
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            );
            if result == ERROR_FILE_NOT_FOUND {
                if !enable {
                    return Ok(());
                }
                check(RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    RUN,
                    0,
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_QUERY_VALUE | KEY_SET_VALUE,
                    None,
                    &mut key,
                    None,
                ))?;
            } else {
                check(result)?;
            }
        }
        update(&Store(key), path, enable)
    }
}

/// Caller must reject elevation and obtain this path from the read-only installation gate.
#[cfg(windows)]
pub fn set(path: &Path, enable: bool) -> Result<(), String> {
    registry::set(path, enable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Store {
        value: RefCell<Option<Value>>,
        writes: Cell<usize>,
    }
    impl RegistryStore for Store {
        fn read(&self) -> Result<Option<Value>, String> {
            Ok(self.value.borrow().clone())
        }
        fn write(&self, value: &Value) -> Result<(), String> {
            self.writes.set(self.writes.get() + 1);
            *self.value.borrow_mut() = Some(value.clone());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }
    #[test]
    fn exact_ownership_and_idempotence() {
        let path = Path::new(r"C:\Program Files\Flower\versions\dynamic\flower-settings.exe");
        let value = expected(path).unwrap();
        assert_eq!(value.kind, 1);
        assert_eq!(value.bytes, "\"C:\\Program Files\\Flower\\versions\\dynamic\\flower-settings.exe\" --startup-input-method\0"
            .encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
        let store = Store::default();
        update(&store, path, false).unwrap();
        update(&store, path, true).unwrap();
        update(&store, path, true).unwrap();
        assert_eq!(store.writes.get(), 1);
        update(&store, path, false).unwrap();
        update(&store, path, false).unwrap();
        assert!(store.read().unwrap().is_none());
        for conflict in [
            Value {
                kind: 2,
                ..value.clone()
            },
            Value {
                kind: 1,
                bytes: vec![0],
            },
            expected(Path::new(r"C:\Other\flower-settings.exe")).unwrap(),
            Value {
                kind: 1,
                bytes: value.bytes[..value.bytes.len() - 2].to_vec(),
            },
        ] {
            *store.value.borrow_mut() = Some(conflict.clone());
            assert!(update(&store, path, true).is_err());
            assert!(update(&store, path, false).is_err());
            assert_eq!(store.read().unwrap(), Some(conflict));
        }
    }
}
