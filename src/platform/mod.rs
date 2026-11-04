//! Platform-specific host integration.

mod candidate;
mod composition;
mod popup;
pub mod windows;

#[cfg(target_os = "windows")]
pub mod settings_ui;

pub use candidate::{
    clamp_candidate_popup, position_candidate_popup, AcceptedCandidateAction, CandidatePopupAction,
    CandidatePopupError, CandidatePopupLayout, CandidatePopupModel, CandidatePopupSnapshot,
    PopupPoint, PopupRect, PopupSize,
};
pub use composition::{CompositionController, CompositionHost};
pub use windows::{is_supported_platform, TEXT_SERVICE_DESCRIPTION};

#[cfg(target_os = "windows")]
pub use windows::{DllCanUnloadNow, DllGetClassObject, FLOWER_TEXT_SERVICE_CLSID};

#[cfg(target_os = "windows")]
pub use popup::{CandidatePopupWindow, PopupError};

#[cfg(test)]
mod tests {
    #[test]
    fn reports_the_current_host_platform() {
        assert_eq!(super::is_supported_platform(), cfg!(target_os = "windows"));
    }
}
