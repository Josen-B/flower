//! 非激活式 Win32 候选弹窗外壳。
//!
//! Windows 实现使用与平台无关的 `CandidatePopupLayout` 同时绘制和命中候选。
//! 鼠标动作先排队，再由自定义窗口消息延迟派发；窗口本身不持有
//! `TextService`，也不运行自己的消息循环。

#[cfg(any(target_os = "windows", test))]
use crate::settings::Theme;

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PopupPalette {
    background: u32,
    text: u32,
    highlight: u32,
    highlight_text: u32,
}

#[cfg(any(target_os = "windows", test))]
const LIGHT_PALETTE: PopupPalette = PopupPalette {
    background: 0x00ff_ffff,
    text: 0x0018_1818,
    highlight: 0x00d7_7800,
    highlight_text: 0x00ff_ffff,
};

#[cfg(any(target_os = "windows", test))]
const DARK_PALETTE: PopupPalette = PopupPalette {
    background: 0x0020_2020,
    text: 0x00f2_f2f2,
    highlight: 0x0000_78d7,
    highlight_text: 0x00ff_ffff,
};

#[cfg(any(target_os = "windows", test))]
pub(crate) fn popup_palette(theme: Theme, system: PopupPalette) -> PopupPalette {
    match theme {
        Theme::System => system,
        Theme::Light => LIGHT_PALETTE,
        Theme::Dark => DARK_PALETTE,
    }
}

/// Convert a validated target-monitor measurement, never an old popup DPI.
#[cfg(any(target_os = "windows", test))]
fn target_candidate_width_budget(work_width: i32, target_dpi: u32) -> u32 {
    if work_width <= 0 || target_dpi == 0 {
        return 544;
    }
    let logical = (work_width as u64 * 96 / u64::from(target_dpi)).min(560);
    logical.saturating_sub(16).clamp(1, 544) as u32
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::rc::Rc;
    use std::sync::{Mutex, MutexGuard};

    use thiserror::Error;
    use windows::core::{Error as WindowsError, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{
        COLORREF, ERROR_CLASS_ALREADY_EXISTS, ERROR_CLASS_DOES_NOT_EXIST, E_UNEXPECTED, HINSTANCE,
        HMODULE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
        GetMonitorInfoW, GetSysColor, InvalidateRect, MonitorFromRect, MonitorFromWindow,
        SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
        COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, DEFAULT_CHARSET,
        DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
        FF_DONTCARE, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, GetDpiForWindow, GetWindowDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
        IsWindowVisible, LoadCursorW, PostMessageW, RegisterClassExW, SetWindowLongPtrW,
        SetWindowPos, ShowWindow, UnregisterClassW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
        GWLP_USERDATA, HCURSOR, HMENU, IDC_ARROW, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SYSCOLORCHANGE,
        WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    use crate::platform::candidate::{
        clamp_candidate_popup, position_candidate_popup, CandidatePopupAction,
        CandidatePopupLayout, PopupPoint, PopupRect, PopupSize,
    };
    use crate::platform::CandidatePopupSnapshot;
    use crate::settings::Theme;

    use super::{popup_palette, target_candidate_width_budget, PopupPalette};

    const POPUP_CLASS_NAME: PCWSTR = windows::core::w!("Flower.CandidatePopupWindow");
    const POPUP_WINDOW_NAME: PCWSTR = windows::core::w!("Flower candidates");
    const FONT_FACE: PCWSTR = windows::core::w!("Microsoft YaHei UI");

    const BASE_DPI: u32 = 96;
    const WM_FLOWER_DISPATCH_ACTION: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

    /// Owns only the temporary built-in hidden window, never the real popup.
    struct HiddenDpiProbe(Option<HWND>);

    impl HiddenDpiProbe {
        fn destroy(&mut self) -> bool {
            match self.0.take() {
                // SAFETY: This guard exclusively owns the window on its creating thread.
                Some(hwnd) => unsafe { DestroyWindow(hwnd) }.is_ok(),
                None => true,
            }
        }
    }

    impl Drop for HiddenDpiProbe {
        fn drop(&mut self) {
            let _ = self.destroy();
        }
    }

    /// A failure while creating, updating, or destroying the candidate popup.
    #[derive(Debug, Error)]
    pub enum PopupError {
        /// No snapshot has been supplied for display.
        #[error("the candidate popup has no snapshot to display")]
        NoSnapshot,

        /// The window has already been destroyed.
        #[error("the candidate popup window has been destroyed")]
        Destroyed,

        /// A Win32 operation failed.
        #[error("candidate popup {operation} failed: {source}")]
        Win32 {
            /// The operation that failed.
            operation: &'static str,
            /// The Win32 or HRESULT error.
            #[source]
            source: WindowsError,
        },
    }

    impl PopupError {
        fn win32(operation: &'static str, source: WindowsError) -> Self {
            Self::Win32 { operation, source }
        }
    }

    fn state_busy_error() -> PopupError {
        PopupError::win32("state borrow", WindowsError::from_hresult(E_UNEXPECTED))
    }

    pub(crate) type PopupActionSink = Rc<dyn Fn(CandidatePopupAction) + 'static>;

    struct PopupState {
        snapshot: Option<CandidatePopupSnapshot>,
        dpi: u32,
        theme: Theme,
        pending_actions: VecDeque<CandidatePopupAction>,
        action_sink: Option<PopupActionSink>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DetachOutcome {
        NotOwned,
        AlreadyDetached,
        Detached,
        Failed,
    }

    struct ClassRegistration {
        references: usize,
    }

    static CLASS_REGISTRATION: Mutex<ClassRegistration> =
        Mutex::new(ClassRegistration { references: 0 });

    struct ClassLease {
        instance: HINSTANCE,
        released: bool,
    }

    impl ClassLease {
        fn acquire(instance: HINSTANCE, cursor: HCURSOR) -> Result<Self, PopupError> {
            let mut registration = lock_class_registration();
            if registration.references == 0 {
                let window_class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(popup_window_proc),
                    hInstance: instance,
                    hCursor: cursor,
                    lpszClassName: POPUP_CLASS_NAME,
                    ..Default::default()
                };

                // SAFETY: `window_class` is fully initialized with the DLL
                // instance that owns `popup_window_proc`, and it remains alive
                // for the registration call. This is a class-only operation;
                // no window creation or destruction happens under the lock.
                let atom = unsafe { RegisterClassExW(&window_class) };
                if atom == 0 {
                    let error = WindowsError::from_win32();
                    let already_exists =
                        error.code() == HRESULT::from_win32(ERROR_CLASS_ALREADY_EXISTS.0);
                    if !already_exists {
                        return Err(PopupError::win32("class registration", error));
                    }
                }
            }

            registration.references = registration.references.saturating_add(1);
            Ok(Self {
                instance,
                released: false,
            })
        }

        fn release(&mut self) -> Result<(), WindowsError> {
            if self.released {
                return Ok(());
            }
            self.released = true;

            let mut registration = lock_class_registration();
            registration.references = registration.references.saturating_sub(1);
            if registration.references != 0 {
                return Ok(());
            }

            // SAFETY: The class name and module instance identify the class
            // registered by `acquire`. This call does not create or destroy a
            // window, so it cannot re-enter popup window creation/destruction.
            match unsafe { UnregisterClassW(POPUP_CLASS_NAME, self.instance) } {
                Ok(()) => Ok(()),
                Err(error) => {
                    let code = error.code();
                    if code == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) {
                        Ok(())
                    } else {
                        // The class (and its function pointer into this DLL)
                        // cannot be confirmed unloaded. Pin this module until
                        // process exit rather than letting DllCanUnloadNow
                        // unload code that may still be referenced.
                        pin_popup_module()?;
                        Err(error)
                    }
                }
            }
        }
    }

    impl Drop for ClassLease {
        fn drop(&mut self) {
            let _ = self.release();
        }
    }

    fn lock_class_registration() -> MutexGuard<'static, ClassRegistration> {
        match CLASS_REGISTRATION.lock() {
            Ok(registration) => registration,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn module_handle(flags: u32) -> Result<HMODULE, WindowsError> {
        let window_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
            popup_window_proc;
        let function_address = (window_proc as *const ()).cast::<u16>();
        let mut module = HMODULE::default();

        // SAFETY: `function_address` points into this DLL, and the caller
        // chooses whether the returned module handle is reference-counted,
        // unchanged, or permanently pinned.
        unsafe { GetModuleHandleExW(flags, PCWSTR::from_raw(function_address), &mut module) }?;

        Ok(module)
    }

    fn popup_module_instance() -> Result<HINSTANCE, PopupError> {
        module_handle(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        )
        .map(HINSTANCE::from)
        .map_err(|error| PopupError::win32("module lookup", error))
    }

    fn pin_popup_module() -> Result<(), WindowsError> {
        module_handle(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN)
            .map(|_| ())
    }

    /// A hidden, non-activating Win32 popup that renders candidate snapshots.
    ///
    /// The window belongs to the thread that creates it. Callers must keep
    /// dispatching messages on that thread for the window to repaint and react
    /// to DPI changes.
    pub struct CandidatePopupWindow {
        hwnd: HWND,
        state: Option<Box<RefCell<PopupState>>>,
        class_lease: Option<ClassLease>,
    }

    impl CandidatePopupWindow {
        /// Registers the popup class and creates a hidden popup window.
        pub fn new() -> Result<Self, PopupError> {
            let instance = popup_module_instance()?;

            // SAFETY: A null instance requests the shared system arrow cursor,
            // and IDC_ARROW is a valid system cursor identifier.
            let cursor = unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW) }
                .map_err(|error| PopupError::win32("cursor load", error))?;

            let class_lease = ClassLease::acquire(instance, cursor)?;

            let state = Box::new(RefCell::new(PopupState {
                snapshot: None,
                dpi: BASE_DPI,
                theme: Theme::System,
                pending_actions: VecDeque::new(),
                action_sink: None,
            }));
            let state_pointer: *const c_void =
                (state.as_ref() as *const RefCell<PopupState>).cast::<c_void>();

            // SAFETY: The class is registered, the optional create parameter is
            // a live pointer to `state`, and the window remains hidden until
            // `show_at` is called. `state` is retained by this struct for the
            // window's entire lifetime.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    POPUP_CLASS_NAME,
                    POPUP_WINDOW_NAME,
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    HWND::default(),
                    HMENU::default(),
                    instance,
                    Some(state_pointer),
                )
            }
            .map_err(|error| PopupError::win32("window creation", error))?;

            Ok(Self {
                hwnd,
                state: Some(state),
                class_lease: Some(class_lease),
            })
        }

        /// Selects the candidate colors used by subsequent repaints.
        ///
        /// `Theme::System` samples current system window, text, and highlight
        /// colors at paint time. It follows system color settings, including
        /// high-contrast values, rather than inferring an app dark mode from
        /// registry settings.
        pub fn set_theme(&mut self, theme: Theme) -> Result<(), PopupError> {
            self.ensure_alive()?;
            {
                let mut state = self
                    .state
                    .as_deref()
                    .ok_or(PopupError::Destroyed)?
                    .try_borrow_mut()
                    .map_err(|_| state_busy_error())?;
                state.theme = theme;
            }

            // SAFETY: The window is live and the null rectangle invalidates the
            // complete client area for the new colors.
            if !unsafe { InvalidateRect(self.hwnd, None, TRUE) }.as_bool() {
                return Err(PopupError::win32(
                    "theme redraw invalidation",
                    WindowsError::from_win32(),
                ));
            }
            Ok(())
        }

        /// 注册或解除鼠标动作回调。
        ///
        /// 回调只接收纯数据动作。窗口销毁、隐藏和失焦都会清除尚未派发的动作；
        /// 回调对象不得反向强持有窗口或其线程局部存储。
        pub(crate) fn set_action_sink(
            &mut self,
            sink: Option<PopupActionSink>,
        ) -> Result<(), PopupError> {
            self.ensure_alive()?;
            let mut state = self
                .state
                .as_deref()
                .ok_or(PopupError::Destroyed)?
                .try_borrow_mut()
                .map_err(|_| state_busy_error())?;
            state.action_sink = sink;
            if state.action_sink.is_none() {
                state.pending_actions.clear();
            }
            Ok(())
        }

        /// 取消已经排队但尚未派发的鼠标动作。
        pub(crate) fn cancel_pending_actions(&mut self) {
            let Some(state) = self.state.as_deref() else {
                return;
            };
            if let Ok(mut state) = state.try_borrow_mut() {
                state.pending_actions.clear();
            }
        }

        /// 替换绘制快照，并按当前 DPI 设置自然尺寸。
        pub fn update(&mut self, snapshot: &CandidatePopupSnapshot) -> Result<(), PopupError> {
            self.ensure_alive()?;

            // SAFETY: `self.hwnd` is a live popup window owned by this struct,
            // and the call has no pointer arguments.
            let dpi = unsafe { GetDpiForWindow(self.hwnd) };
            let dpi = if dpi == 0 { BASE_DPI } else { dpi };
            let natural =
                CandidatePopupLayout::new(snapshot, dpi, PopupSize::new(i32::MAX, i32::MAX)).size;

            // SAFETY: The window is live. Resizing does not activate it, and a
            // null insert-after handle is ignored because SWP_NOZORDER is set.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND::default(),
                    0,
                    0,
                    natural.width,
                    natural.height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| PopupError::win32("resize", error))?;

            {
                let mut state = self
                    .state
                    .as_deref()
                    .ok_or(PopupError::Destroyed)?
                    .try_borrow_mut()
                    .map_err(|_| state_busy_error())?;
                state.snapshot = Some(snapshot.clone());
                state.dpi = dpi;
                state.pending_actions.clear();
            }

            // SAFETY: The window is live and the null rectangle invalidates the
            // complete client area.
            let invalidated = unsafe { InvalidateRect(self.hwnd, None, TRUE) };
            if !invalidated.as_bool() {
                return Err(PopupError::win32(
                    "redraw invalidation",
                    WindowsError::from_win32(),
                ));
            }

            Ok(())
        }

        /// 查询插入符目标显示器的逻辑内容宽预算。
        /// 跨屏时短暂创建独立隐藏探针；不移动弹窗、不更改宿主或线程 DPI。
        /// 仅供成功编辑后的发布路径使用，不用于按键试探。失败保留默认预算。
        pub fn candidate_width_budget(&self, caret: PopupRect) -> Result<u32, PopupError> {
            self.ensure_alive()?;
            let native = windows_rect(caret);
            // SAFETY: The rectangle is valid storage and neither query mutates windows.
            let target = unsafe { MonitorFromRect(&native, MONITOR_DEFAULTTONEAREST) };
            if target.is_invalid() {
                return Ok(544);
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: The monitor is valid and the output structure has its required size.
            if !unsafe { GetMonitorInfoW(target, &mut info) }.as_bool() {
                return Ok(544);
            }
            // SAFETY: `self.hwnd` is live. Only a same-monitor popup supplies target DPI.
            let dpi = if unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) } == target
            {
                unsafe { GetDpiForWindow(self.hwnd) }
            } else {
                // Built-in STATIC needs no custom class or callback. No WS_VISIBLE,
                // owner, parent, focus API, or awareness mutation is involved.
                let probe = unsafe {
                    CreateWindowExW(
                        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                        windows::core::w!("STATIC"),
                        PCWSTR::null(),
                        WS_POPUP,
                        caret.left,
                        caret.top,
                        1,
                        1,
                        HWND::default(),
                        HMENU::default(),
                        HINSTANCE::default(),
                        None,
                    )
                };
                let Ok(probe) = probe else {
                    return Ok(544);
                };
                let mut guard = HiddenDpiProbe(Some(probe));
                // SAFETY: Both windows are live. The guard destroys the probe on
                // every return path, including monitor/context validation failure.
                let valid = unsafe {
                    let probe_context = GetWindowDpiAwarenessContext(probe);
                    let popup_context = GetWindowDpiAwarenessContext(self.hwnd);
                    MonitorFromWindow(probe, MONITOR_DEFAULTTONEAREST) == target
                        && !probe_context.0.is_null()
                        && !popup_context.0.is_null()
                        && AreDpiAwarenessContextsEqual(probe_context, popup_context).as_bool()
                };
                if !valid {
                    return Ok(544);
                }
                let dpi = unsafe { GetDpiForWindow(probe) };
                if !guard.destroy() {
                    return Ok(544);
                }
                dpi
            };
            Ok(target_candidate_width_budget(
                rect_from_windows(info.rcWork).width(),
                dpi,
            ))
        }

        /// 根据插入符位置选择显示器，重算目标 DPI 布局后再显示。
        ///
        /// 弹窗不调用前台或焦点 API。跨显示器时先把隐藏窗口移动到目标显示器，
        /// 让 `GetDpiForWindow` 返回目标 DPI，再按工作区限制尺寸。
        pub fn show_at(&self, caret: PopupRect) -> Result<(), PopupError> {
            self.ensure_alive()?;
            let snapshot = self.snapshot().ok_or(PopupError::NoSnapshot)?;
            self.hide();
            let native_caret = RECT {
                left: caret.left,
                top: caret.top,
                right: caret.right,
                bottom: caret.bottom,
            };

            // SAFETY: `native_caret` is live for the query. The nearest monitor
            // fallback covers caret rectangles outside a particular monitor.
            let monitor = unsafe { MonitorFromRect(&native_caret, MONITOR_DEFAULTTONEAREST) };
            if monitor.is_invalid() {
                return Err(PopupError::win32(
                    "monitor lookup",
                    WindowsError::from_win32(),
                ));
            }
            let mut monitor_info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: The monitor handle came from `MonitorFromRect`, and
            // `monitor_info` is a valid output structure with `cbSize` set.
            if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
                return Err(PopupError::win32(
                    "monitor work area",
                    WindowsError::from_win32(),
                ));
            }
            let work_area = rect_from_windows(monitor_info.rcWork);

            // Move while hidden with a temporary size that fits the selected
            // monitor. This guarantees that `GetDpiForWindow` observes the
            // target monitor rather than an adjacent monitor at the caret edge.
            let temporary_layout = CandidatePopupLayout::new(
                &snapshot,
                BASE_DPI,
                PopupSize::new(work_area.width(), work_area.height()),
            );
            let temporary = position_candidate_popup(caret, temporary_layout.size, work_area)
                .ok_or_else(|| {
                    PopupError::win32(
                        "temporary placement",
                        WindowsError::from_hresult(E_UNEXPECTED),
                    )
                })?;
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND::default(),
                    temporary.left,
                    temporary.top,
                    temporary.width(),
                    temporary.height(),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| PopupError::win32("target monitor move", error))?;
            let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(BASE_DPI);
            let layout = CandidatePopupLayout::new(
                &snapshot,
                dpi,
                PopupSize::new(work_area.width(), work_area.height()),
            );
            let placement =
                position_candidate_popup(caret, layout.size, work_area).ok_or_else(|| {
                    PopupError::win32("empty work area", WindowsError::from_hresult(E_UNEXPECTED))
                })?;

            {
                let state = self
                    .state
                    .as_deref()
                    .ok_or(PopupError::Destroyed)?
                    .try_borrow()
                    .map_err(|_| state_busy_error())?;
                if state.snapshot.as_ref() != Some(&snapshot) {
                    return Err(PopupError::NoSnapshot);
                }
            }
            {
                let mut state = self
                    .state
                    .as_deref()
                    .ok_or(PopupError::Destroyed)?
                    .try_borrow_mut()
                    .map_err(|_| state_busy_error())?;
                state.dpi = dpi;
            }

            // SAFETY: The window is live. SWP_SHOWWINDOW shows it without
            // activation, and the placement is already clipped to the monitor
            // work area. No foreground or focus API is called.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND::default(),
                    placement.left,
                    placement.top,
                    layout.size.width,
                    layout.size.height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            }
            .map_err(|error| PopupError::win32("show", error))?;

            // SAFETY: The window is live and the null rectangle invalidates the
            // complete client area after any cross-monitor DPI change.
            if !unsafe { InvalidateRect(self.hwnd, None, TRUE) }.as_bool() {
                return Err(PopupError::win32(
                    "show invalidation",
                    WindowsError::from_win32(),
                ));
            }

            Ok(())
        }

        fn snapshot(&self) -> Option<CandidatePopupSnapshot> {
            let state = self.state.as_deref()?;
            let state = state.try_borrow().ok()?;
            state.snapshot.clone()
        }

        /// 隐藏窗口并取消仍在排队的点击。
        pub fn hide(&self) {
            let Some(state) = self.state.as_deref() else {
                return;
            };
            if let Ok(mut state) = state.try_borrow_mut() {
                state.pending_actions.clear();
            }
            if self.ensure_alive().is_err() {
                return;
            }

            // SAFETY: The handle belongs to this live popup. SW_HIDE hides the
            // window without changing keyboard focus.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }

        /// Reports whether the popup is currently visible.
        pub fn is_visible(&self) -> bool {
            if self.ensure_alive().is_err() {
                return false;
            }

            // SAFETY: The handle belongs to this popup; the query has no
            // pointer arguments and does not change window state.
            unsafe { IsWindowVisible(self.hwnd).as_bool() }
        }

        /// Destroys the native window while retaining the Rust-owned state.
        pub fn destroy(&mut self) -> Result<(), PopupError> {
            if self.hwnd.is_invalid() {
                self.state = None;
                return self.release_class_lease();
            }

            let hwnd = self.hwnd;
            let state_pointer = self.state_pointer();
            match unsafe { detach_popup_state(hwnd, state_pointer) } {
                DetachOutcome::NotOwned | DetachOutcome::AlreadyDetached => {
                    // The handle was already destroyed, detached, or recycled
                    // by an unrelated window. Never call destructive window
                    // APIs for it.
                    self.hwnd = HWND::default();
                    self.state = None;
                    self.release_class_lease()
                }
                DetachOutcome::Failed => Err(PopupError::win32(
                    "user data detach",
                    WindowsError::from_win32(),
                )),
                DetachOutcome::Detached => {
                    // SAFETY: Ownership was confirmed by `detach_popup_state`,
                    // which also cleared GWLP_USERDATA before this call.
                    let destroy_result = unsafe { DestroyWindow(hwnd) };
                    self.hwnd = HWND::default();
                    self.state = None;
                    let class_result = self.release_class_lease();
                    match destroy_result {
                        Ok(()) => class_result,
                        Err(error) => Err(PopupError::win32("destroy", error)),
                    }
                }
            }
        }

        fn ensure_alive(&self) -> Result<(), PopupError> {
            if self.window_is_owned() {
                Ok(())
            } else {
                Err(PopupError::Destroyed)
            }
        }

        fn window_is_owned(&self) -> bool {
            if self.hwnd.is_invalid() {
                return false;
            }
            let Some(state) = self.state.as_deref() else {
                return false;
            };
            let expected = state as *const RefCell<PopupState>;

            // SAFETY: This is a read-only identity check against the window's
            // user data. It avoids any destructive operation on a recycled
            // handle that no longer belongs to this object.
            let current = unsafe { popup_state_pointer(self.hwnd) } as *const RefCell<PopupState>;
            std::ptr::eq(current, expected)
        }

        fn state_pointer(&self) -> *const RefCell<PopupState> {
            self.state.as_deref().map_or(std::ptr::null(), |state| {
                state as *const RefCell<PopupState>
            })
        }

        fn release_class_lease(&mut self) -> Result<(), PopupError> {
            let Some(mut class_lease) = self.class_lease.take() else {
                return Ok(());
            };
            class_lease
                .release()
                .map_err(|error| PopupError::win32("class unregister", error))
        }
    }

    impl Drop for CandidatePopupWindow {
        fn drop(&mut self) {
            if !self.hwnd.is_invalid() {
                let hwnd = self.hwnd;
                let state_pointer = self.state_pointer();
                match unsafe { detach_popup_state(hwnd, state_pointer) } {
                    DetachOutcome::NotOwned | DetachOutcome::AlreadyDetached => {
                        self.state = None;
                    }
                    DetachOutcome::Failed => {
                        if let Some(state) = self.state.take() {
                            // The window may still reference this state, and
                            // the association could not be cleared. Leaking it
                            // is the only safe fallback.
                            std::mem::forget(state);
                        }
                    }
                    DetachOutcome::Detached => {
                        // SAFETY: Ownership was confirmed and the state pointer
                        // was cleared, so destroying the confirmed window
                        // cannot race a stale WndProc state pointer.
                        let _ = unsafe { DestroyWindow(hwnd) };
                        self.state = None;
                    }
                }
                self.hwnd = HWND::default();
            } else {
                self.state = None;
            }

            let _ = self.release_class_lease();
        }
    }

    unsafe extern "system" fn popup_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: Windows invokes this procedure only for live windows of
            // the registered popup class. The raw state pointer is non-owning
            // and remains valid until the owning struct is dropped.
            unsafe { handle_popup_message(hwnd, message, wparam, lparam) }
        })) {
            Ok(result) => result,
            Err(_) => {
                // SAFETY: This fallback uses the same live window arguments.
                // Letting DefWindowProcW handle the message keeps panics from
                // unwinding across the Windows ABI.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }

    unsafe fn handle_popup_message(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_NCCREATE => {
                if lparam.0 != 0 {
                    let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
                    // SAFETY: The creation parameter is the non-owning state
                    // pointer supplied by `CandidatePopupWindow::new`.
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                    }
                }
                // SAFETY: Forward the live creation message to the default
                // procedure so window creation can complete normally.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
            WM_LBUTTONDOWN => LRESULT(0),
            WM_LBUTTONUP => {
                let point = point_from_lparam(lparam);
                if let Some(action) = popup_action_at(hwnd, point) {
                    let _ = queue_popup_action(hwnd, action);
                }
                LRESULT(0)
            }
            WM_FLOWER_DISPATCH_ACTION => {
                dispatch_one_popup_action(hwnd);
                LRESULT(0)
            }
            WM_PAINT => {
                let render_data = popup_render_data(hwnd);
                paint_popup(hwnd, render_data.as_ref());
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SYSCOLORCHANGE => {
                // System palettes are sampled during painting, so a system
                // color change only needs a repaint.
                // SAFETY: The window is live and the null rectangle invalidates
                // the complete client area.
                let _ = unsafe { InvalidateRect(hwnd, None, TRUE) };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let new_dpi = (wparam.0 & 0xffff) as u32;
                let snapshot = {
                    // SAFETY: The user data slot is either null or points to
                    // the live RefCell retained by the owning Rust struct.
                    unsafe { popup_state_pointer(hwnd).as_ref() }.and_then(|state| {
                        if let Ok(mut state) = state.try_borrow_mut() {
                            state.dpi = new_dpi.max(1);
                            state.snapshot.clone()
                        } else {
                            None
                        }
                    })
                };

                if lparam.0 != 0 {
                    // SAFETY: WM_DPICHANGED supplies a pointer to a RECT that is
                    // valid for the duration of this message.
                    let suggested = unsafe { *(lparam.0 as *const RECT) };
                    if let Some(snapshot) = snapshot.as_ref() {
                        let work_area = monitor_work_area(rect_from_windows(suggested));
                        if let Some(work_area) = work_area {
                            let layout = CandidatePopupLayout::new(
                                snapshot,
                                new_dpi,
                                PopupSize::new(work_area.width(), work_area.height()),
                            );
                            let requested = PopupRect::new(
                                suggested.left,
                                suggested.top,
                                suggested.left.saturating_add(layout.size.width),
                                suggested.top.saturating_add(layout.size.height),
                            );
                            if let Some(placement) = clamp_candidate_popup(requested, work_area) {
                                // SAFETY: The window is live. The suggested
                                // rectangle is applied without activation.
                                let _ = unsafe {
                                    SetWindowPos(
                                        hwnd,
                                        HWND::default(),
                                        placement.left,
                                        placement.top,
                                        placement.width(),
                                        placement.height(),
                                        SWP_NOZORDER | SWP_NOACTIVATE,
                                    )
                                };
                            }
                        }
                    }
                }

                // SAFETY: The window is live and the null rectangle invalidates
                // the complete client area after the DPI change.
                let _ = unsafe { InvalidateRect(hwnd, None, TRUE) };
                LRESULT(0)
            }
            WM_NCDESTROY => {
                // SAFETY: The user data slot is either null or points to the
                // live RefCell retained by the owning Rust struct.
                if let Some(state) = unsafe { popup_state_pointer(hwnd).as_ref() } {
                    if let Ok(mut state) = state.try_borrow_mut() {
                        state.pending_actions.clear();
                        state.action_sink = None;
                    }
                }
                // SAFETY: The window is being destroyed. Clear the non-owning
                // state pointer before forwarding destruction to the default
                // procedure; the Box remains owned by CandidatePopupWindow.
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                // SAFETY: Forward the live destruction message and its original
                // arguments to the default procedure.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
            _ => {
                // SAFETY: Forward unhandled live messages to the default
                // procedure.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }

    unsafe fn popup_state_pointer(hwnd: HWND) -> *mut RefCell<PopupState> {
        // SAFETY: This reads the window's user data slot for a live window.
        let value = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
        value as *mut RefCell<PopupState>
    }

    fn point_from_lparam(lparam: LPARAM) -> PopupPoint {
        let packed = lparam.0 as u32;
        PopupPoint {
            x: i32::from((packed as u16) as i16),
            y: i32::from(((packed >> 16) as u16) as i16),
        }
    }

    fn rect_from_windows(rect: RECT) -> PopupRect {
        PopupRect::new(rect.left, rect.top, rect.right, rect.bottom)
    }

    fn monitor_work_area(rect: PopupRect) -> Option<PopupRect> {
        let native = RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        // SAFETY: `native` is valid for the duration of the query.
        let monitor = unsafe { MonitorFromRect(&native, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: The monitor handle is live, and `info` is writable with its
        // required size initialized.
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return None;
        }
        Some(rect_from_windows(info.rcWork))
    }

    fn popup_action_at(hwnd: HWND, point: PopupPoint) -> Option<CandidatePopupAction> {
        let mut client = RECT::default();
        // SAFETY: `hwnd` is live and `client` is valid writable storage.
        if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
            return None;
        }

        // SAFETY: The user data slot is either null or points to the live
        // RefCell retained by the owning Rust struct.
        let state = unsafe { popup_state_pointer(hwnd).as_ref()? };
        let state = state.try_borrow().ok()?;
        let snapshot = state.snapshot.as_ref()?;
        let layout = CandidatePopupLayout::new(
            snapshot,
            state.dpi,
            PopupSize::new(client.right - client.left, client.bottom - client.top),
        );

        // Win32 must not be called while the RefCell borrow is held. The layout
        // and hit test are pure Rust operations.
        layout.hit_test(snapshot, point)
    }

    fn queue_popup_action(hwnd: HWND, action: CandidatePopupAction) -> bool {
        let queued = {
            // SAFETY: The user data slot is either null or points to the live
            // RefCell retained by the owning Rust struct.
            let Some(state) = (unsafe { popup_state_pointer(hwnd).as_ref() }) else {
                return false;
            };
            let Ok(mut state) = state.try_borrow_mut() else {
                return false;
            };
            if state.action_sink.is_none() {
                false
            } else {
                state.pending_actions.push_back(action);
                true
            }
        };
        if !queued {
            return false;
        }

        // SAFETY: The window is live. A custom message defers the callback until
        // WM_LBUTTONUP handling has completely returned.
        unsafe {
            PostMessageW(
                hwnd,
                WM_FLOWER_DISPATCH_ACTION,
                WPARAM::default(),
                LPARAM::default(),
            )
        }
        .is_ok()
    }

    fn dispatch_one_popup_action(hwnd: HWND) {
        let (action, sink, has_more) = {
            // SAFETY: The user data slot is either null or points to the live
            // RefCell retained by the owning Rust struct.
            let Some(state) = (unsafe { popup_state_pointer(hwnd).as_ref() }) else {
                return;
            };
            let Ok(mut state) = state.try_borrow_mut() else {
                return;
            };
            let action = state.pending_actions.pop_front();
            let sink = state.action_sink.clone();
            let has_more = !state.pending_actions.is_empty();
            (action, sink, has_more)
        };

        if let (Some(action), Some(sink)) = (action, sink) {
            // The RefCell borrow has ended. The sink only starts asynchronous
            // TSF work and must not perform an edit while inside WndProc.
            sink(action);
        }
        if has_more {
            // SAFETY: The window is live; defer the next queued action.
            let _ = unsafe {
                PostMessageW(
                    hwnd,
                    WM_FLOWER_DISPATCH_ACTION,
                    WPARAM::default(),
                    LPARAM::default(),
                )
            };
        }
    }

    unsafe fn detach_popup_state(
        hwnd: HWND,
        expected: *const RefCell<PopupState>,
    ) -> DetachOutcome {
        if expected.is_null() || hwnd.is_invalid() {
            return DetachOutcome::NotOwned;
        }

        // SAFETY: This reads the live window's user data slot. The expected
        // pointer comparison avoids writing to a recycled, unrelated HWND.
        let current = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
        if current != expected as isize {
            return if current == 0 {
                DetachOutcome::AlreadyDetached
            } else {
                DetachOutcome::NotOwned
            };
        }

        // SAFETY: The slot still contains exactly the state pointer owned by
        // this object. Clearing it prevents a later WndProc from observing the
        // state after the Box is released.
        if unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) == expected as isize } {
            DetachOutcome::Detached
        } else {
            DetachOutcome::Failed
        }
    }

    fn popup_render_data(hwnd: HWND) -> Option<(CandidatePopupSnapshot, u32, Theme)> {
        // SAFETY: The user data slot is either null or points to the live
        // RefCell retained by the owning Rust struct.
        let state = unsafe { popup_state_pointer(hwnd).as_ref()? };
        let state = state.try_borrow().ok()?;
        let snapshot = state.snapshot.clone()?;
        let dpi = state.dpi.max(1);
        let theme = state.theme;

        // The borrow ends here, before any paint or other Win32 call.
        Some((snapshot, dpi, theme))
    }

    fn paint_popup(hwnd: HWND, render_data: Option<&(CandidatePopupSnapshot, u32, Theme)>) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: `hwnd` is live and `paint` is valid writable storage.
        let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
        if hdc.is_invalid() {
            return;
        }

        let fallback = paint.rcPaint;
        let _paint_guard = PaintGuard { hwnd, paint };

        let mut client = RECT::default();
        // SAFETY: `hwnd` is live and `client` is valid writable storage.
        if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
            client = fallback;
        }

        let Some((snapshot, dpi, theme)) = render_data else {
            fill_rect(hdc, &client, COLORREF(system_popup_palette().background));
            return;
        };
        let palette = popup_palette(*theme, system_popup_palette());
        fill_rect(hdc, &client, COLORREF(palette.background));
        let dpi = *dpi;
        let font = PaintFont::new(hdc, dpi);

        // SAFETY: The paint device context is live for the duration of this
        // function, and the font, colors, and text buffers stay alive for each
        // corresponding call.
        unsafe {
            SetBkMode(hdc, TRANSPARENT);
        }

        let layout = CandidatePopupLayout::new(
            snapshot,
            dpi,
            PopupSize::new(client.right - client.left, client.bottom - client.top),
        );
        for (index, (row, candidate)) in layout
            .candidates
            .iter()
            .zip(snapshot.candidates.iter())
            .enumerate()
        {
            let mut row_rect = windows_rect(*row);
            let highlighted = snapshot.highlighted == Some(index);
            if highlighted {
                fill_rect(hdc, &row_rect, COLORREF(palette.highlight));
                set_text_color(hdc, COLORREF(palette.highlight_text));
            } else {
                set_text_color(hdc, COLORREF(palette.text));
            }

            let line = format!("{}. {}", index + 1, candidate.text);
            draw_text(
                hdc,
                &line,
                &mut row_rect,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }

        set_text_color(hdc, COLORREF(palette.text));
        for (button, label) in [(layout.previous_page, "-"), (layout.next_page, "+")] {
            if let Some(button) = button {
                let mut rect = windows_rect(button);
                draw_text(
                    hdc,
                    label,
                    &mut rect,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
            }
        }
        if let Some(page_label_rect) = layout.page_label {
            let mut page_label_rect = windows_rect(page_label_rect);
            let page_label = format!(
                "{}/{}",
                snapshot.page.saturating_add(1),
                snapshot.page_count
            );
            draw_text(
                hdc,
                &page_label,
                &mut page_label_rect,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }

        drop(font);
    }

    fn system_popup_palette() -> PopupPalette {
        // SAFETY: These queries have no pointer arguments and read the current
        // process's system color table. They do not read Flower's config.
        unsafe {
            PopupPalette {
                background: GetSysColor(COLOR_WINDOW),
                text: GetSysColor(COLOR_WINDOWTEXT),
                highlight: GetSysColor(COLOR_HIGHLIGHT),
                highlight_text: GetSysColor(COLOR_HIGHLIGHTTEXT),
            }
        }
    }

    struct PaintGuard {
        hwnd: HWND,
        paint: PAINTSTRUCT,
    }

    impl Drop for PaintGuard {
        fn drop(&mut self) {
            // SAFETY: BeginPaint succeeded for this live window, and this guard
            // releases that paint exactly once.
            let _ = unsafe { EndPaint(self.hwnd, &self.paint) };
        }
    }

    struct PaintFont {
        hdc: HDC,
        font: HFONT,
        previous: HGDIOBJ,
    }

    impl PaintFont {
        fn new(hdc: HDC, dpi: u32) -> Option<Self> {
            // SAFETY: CreateFontW has no pointer parameters beyond the static
            // null-terminated face name, which remains valid for the call.
            let font = unsafe {
                CreateFontW(
                    -scaled(18, dpi),
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET.0 as u32,
                    OUT_DEFAULT_PRECIS.0 as u32,
                    CLIP_DEFAULT_PRECIS.0 as u32,
                    CLEARTYPE_QUALITY.0 as u32,
                    (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                    FONT_FACE,
                )
            };
            if font.is_invalid() {
                return None;
            }

            // SAFETY: The paint device context is live and the font handle was
            // just created. The previous object is restored in Drop.
            let previous = unsafe { SelectObject(hdc, font) };
            Some(Self {
                hdc,
                font,
                previous,
            })
        }
    }

    impl Drop for PaintFont {
        fn drop(&mut self) {
            // SAFETY: This guard owns the selected GDI font for a live paint
            // device context. Restore the previous object before deleting it.
            unsafe {
                if !self.previous.is_invalid() {
                    let _ = SelectObject(self.hdc, self.previous);
                }
                if !self.font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ::from(self.font));
                }
            }
        }
    }

    fn draw_text(
        hdc: HDC,
        text: &str,
        rect: &mut RECT,
        format: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
    ) {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        if wide.is_empty() {
            return;
        }

        // SAFETY: Both the paint device context and rectangle are live, and
        // `wide` remains valid for the duration of the call.
        unsafe {
            DrawTextW(hdc, &mut wide, rect, format);
        }
    }

    fn fill_rect(hdc: HDC, rect: &RECT, color: COLORREF) {
        // SAFETY: CreateSolidBrush has no pointer arguments.
        let brush: HBRUSH = unsafe { CreateSolidBrush(color) };
        if brush.is_invalid() {
            return;
        }

        // SAFETY: The device context and rectangle are live. The brush is
        // deleted below after all uses.
        unsafe {
            let _ = FillRect(hdc, rect, brush);
            let _ = DeleteObject(HGDIOBJ::from(brush));
        }
    }

    fn set_text_color(hdc: HDC, color: COLORREF) {
        // SAFETY: The paint device context remains live until EndPaint.
        unsafe {
            SetTextColor(hdc, color);
        }
    }

    fn windows_rect(rect: PopupRect) -> RECT {
        RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn scaled(value: i32, dpi: u32) -> i32 {
        let dpi = if dpi == 0 { BASE_DPI } else { dpi.min(480) };
        ((i64::from(value) * i64::from(dpi) + i64::from(BASE_DPI / 2)) / i64::from(BASE_DPI)) as i32
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{CandidatePopupWindow, PopupError};

#[cfg(test)]
mod tests {
    use super::{popup_palette, PopupPalette, DARK_PALETTE, LIGHT_PALETTE};
    use crate::settings::Theme;

    #[test]
    fn target_budget_uses_target_dpi_and_clamps_or_falls_back() {
        use super::target_candidate_width_budget as budget;
        // Same physical work area, two possible destination screens.
        assert_eq!(budget(800, 96), 544);
        assert_eq!(budget(800, 192), 384);
        // There is deliberately no old-window DPI input: going in either
        // direction must produce the destination's budget.
        for (target_dpi, expected) in [(192, 384), (96, 544), (192, 384)] {
            assert_eq!(budget(800, target_dpi), expected);
        }
        assert_eq!(budget(1120, 192), 544);
        assert_eq!(budget(1, 192), 1);
        assert_eq!(budget(0, 192), 544);
        assert_eq!(budget(800, 0), 544);
        assert_eq!(budget(i32::MAX, 96), 544);
    }

    fn system_palette() -> PopupPalette {
        PopupPalette {
            background: 1,
            text: 2,
            highlight: 3,
            highlight_text: 4,
        }
    }

    #[test]
    fn fixed_themes_select_their_palette() {
        assert_eq!(popup_palette(Theme::Light, system_palette()), LIGHT_PALETTE);
        assert_eq!(popup_palette(Theme::Dark, system_palette()), DARK_PALETTE);
    }

    #[test]
    fn system_theme_uses_supplied_system_colors() {
        assert_eq!(
            popup_palette(Theme::System, system_palette()),
            system_palette()
        );
    }
}
