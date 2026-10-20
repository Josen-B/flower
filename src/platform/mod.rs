//! Platform-specific host integration.

pub mod windows;

pub use windows::{is_supported_platform, TEXT_SERVICE_DESCRIPTION};

#[cfg(target_os = "windows")]
pub use windows::{DllCanUnloadNow, DllGetClassObject, FLOWER_TEXT_SERVICE_CLSID};

#[cfg(test)]
mod tests {
    #[test]
    fn reports_the_current_host_platform() {
        assert_eq!(super::is_supported_platform(), cfg!(target_os = "windows"));
    }
}
