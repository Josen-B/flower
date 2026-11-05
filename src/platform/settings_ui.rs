//! 纯 Rust/Win32 的 Flower 设置窗口。
//!
//! 此模块只由独立的 `flower-settings` 可执行文件使用。窗口拥有自己的
//! 消息循环，不创建托盘或后台线程，也不影响 DLL 中 TSF 服务的宿主消息循环。

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

use windows::core::{Error as WindowsError, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, FALSE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetSysColor, GetSysColorBrush, SetBkMode, SetTextColor,
    UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_BTNTEXT,
    DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HDC, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetDlgCtrlID, GetMessageW,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, IsWindow,
    LoadCursorW, MessageBoxW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassExW,
    SendMessageW, SetDlgItemTextW, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    UnregisterClassW, BN_CLICKED, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CBS_DROPDOWNLIST, CB_ADDSTRING,
    CB_ERR, CB_GETDROPPEDSTATE, CB_SETCURSEL, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    ES_AUTOHSCROLL, ES_NUMBER, GWLP_USERDATA, HMENU, IDCANCEL, IDC_ARROW, IDOK, IDYES,
    MB_ICONERROR, MB_OK, MB_YESNO, MSG, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_NCCREATE, WM_NCDESTROY,
    WM_SYSCOLORCHANGE, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_CONTROLPARENT, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::settings::{
    parse_candidate_page_size_input, Settings, SettingsStore, Theme, MAX_CANDIDATE_PAGE_SIZE,
    MIN_CANDIDATE_PAGE_SIZE,
};
use crate::user_dictionary::DictionaryStore;

const WINDOW_CLASS_NAME: PCWSTR = windows::core::w!("Flower.SettingsWindow");
const WINDOW_TITLE: PCWSTR = windows::core::w!("Flower 设置");
const FONT_FACE: PCWSTR = windows::core::w!("Microsoft YaHei UI");
const BASE_DPI: u32 = 96;
const CLIENT_WIDTH: i32 = 460;
const CLIENT_HEIGHT: i32 = 580;

const PAGE_LABEL_ID: i32 = 100;
const PAGE_EDIT_ID: i32 = 101;
const PAGE_HINT_ID: i32 = 102;
const THEME_LABEL_ID: i32 = 103;
const THEME_COMBO_ID: i32 = 104;
const NOTICE_ID: i32 = 105;
const LEARNING_ID: i32 = 106;
const CLEAR_LEARNING_ID: i32 = 107;
const DOMAIN_COMBO_ID: i32 = 108;
const DOMAIN_ENABLED_ID: i32 = 109;
const DICTIONARY_STATUS_ID: i32 = 110;
const SAVE_BUTTON_ID: i32 = IDOK.0;
const CANCEL_BUTTON_ID: i32 = IDCANCEL.0;

/// Runs the settings window on the current thread and returns after it closes.
pub fn run_settings_app() -> Result<(), String> {
    let store = match SettingsStore::for_current_user() {
        Ok(store) => store,
        Err(error) => {
            let message = format!("无法确定当前用户的设置路径：{error}");
            message_box(
                HWND::default(),
                &message,
                "Flower 设置",
                MB_OK | MB_ICONERROR,
            );
            return Err(message);
        }
    };
    let settings = match store.load() {
        Ok(settings) => settings,
        Err(error) => {
            let message = format!(
                "无法读取设置文件：{error}\n\n路径：{}\n\n是否使用默认设置继续？\n\
                 选择“否”将退出，并且不会覆盖现有文件。",
                store.path().display()
            );
            if message_box(
                HWND::default(),
                &message,
                "Flower 设置",
                MB_YESNO | MB_ICONERROR,
            ) != IDYES
            {
                return Ok(());
            }
            Settings::default()
        }
    };

    // SAFETY: The executable module handle is used only to register and create
    // this process's settings window. The cursor is the shared system arrow.
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map(HINSTANCE::from)
        .map_err(|error| format!("无法获取设置程序模块：{error}"))?;
    let cursor = unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW) }
        .map_err(|error| format!("无法加载窗口光标：{error}"))?;
    let background = unsafe { GetSysColorBrush(COLOR_BTNFACE) };
    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(settings_window_proc),
        hInstance: instance,
        hCursor: cursor,
        hbrBackground: background,
        lpszClassName: WINDOW_CLASS_NAME,
        ..Default::default()
    };

    // SAFETY: `window_class` is fully initialized and points only to static
    // names and this module's window procedure.
    if unsafe { RegisterClassExW(&window_class) } == 0 {
        return Err(format!(
            "无法注册设置窗口类：{}",
            WindowsError::from_win32()
        ));
    }

    let state = Box::new(RefCell::new(SettingsUiState {
        store,
        settings,
        controls: SettingsControls::default(),
        fonts: Vec::new(),
        saving: false,
        creation_error: None,
        domain_names: Vec::new(),
    }));
    let state_pointer = state_pointer(state.as_ref());
    let dpi = unsafe { GetDpiForSystem() }.max(BASE_DPI);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut window_rect = RECT {
        left: 0,
        top: 0,
        right: scaled(CLIENT_WIDTH, dpi),
        bottom: scaled(CLIENT_HEIGHT, dpi),
    };
    let adjusted = unsafe {
        AdjustWindowRectExForDpi(&mut window_rect, style, FALSE, WS_EX_CONTROLPARENT, dpi)
    };
    if let Err(error) = adjusted {
        drop(state);
        let _ = unsafe { UnregisterClassW(WINDOW_CLASS_NAME, instance) };
        return Err(format!("无法计算设置窗口尺寸：{error}"));
    }

    // SAFETY: The class is registered, the create parameter points to `state`
    // for the duration of the message loop, and the window starts hidden.
    let created = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            WINDOW_CLASS_NAME,
            WINDOW_TITLE,
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            window_rect.right - window_rect.left,
            window_rect.bottom - window_rect.top,
            HWND::default(),
            HMENU::default(),
            instance,
            Some(state_pointer),
        )
    };
    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let detail = state
                .try_borrow_mut()
                .ok()
                .and_then(|mut state| state.creation_error.take())
                .unwrap_or_else(|| format!("{error}"));
            delete_all_fonts(state.as_ref());
            drop(state);
            let _ = unsafe { UnregisterClassW(WINDOW_CLASS_NAME, instance) };
            return Err(format!("无法创建设置窗口：{detail}"));
        }
    };

    unsafe {
        // ShowWindow reports the previous visibility state, not success.
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = UpdateWindow(hwnd);
    }
    let page_edit = state
        .try_borrow()
        .ok()
        .map(|state| state.controls.page_edit)
        .unwrap_or_default();
    if !page_edit.is_invalid() {
        let _ = unsafe { SetFocus(page_edit) };
    }

    let mut message = MSG::default();
    let loop_result = loop {
        // SAFETY: `message` is writable storage for the thread message queue.
        let result = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) };
        if result.0 == -1 {
            break Err("读取设置窗口消息失败".to_owned());
        }
        if result.0 == 0 {
            break Ok(());
        }

        // SAFETY: `hwnd` is the live top-level control parent and `message`
        // belongs to this thread. IsDialogMessage provides Tab navigation.
        if unsafe { IsDialogMessageW(hwnd, &message) }.as_bool() {
            continue;
        }
        if message.message == WM_KEYDOWN
            && window_is_owned(hwnd, state.as_ref())
            && !theme_dropdown_open(state.as_ref())
        {
            let command = match message.wParam.0 as u32 {
                key if key == VK_ESCAPE.0 as u32 => Some(CANCEL_BUTTON_ID),
                key if key == VK_RETURN.0 as u32 => {
                    let focus = unsafe { GetFocus() };
                    let focused_id = if focus.is_invalid() {
                        0
                    } else {
                        unsafe { GetDlgCtrlID(focus) }
                    };
                    let command = if matches!(
                        focused_id,
                        CANCEL_BUTTON_ID | CLEAR_LEARNING_ID | LEARNING_ID | DOMAIN_ENABLED_ID
                    ) {
                        focused_id
                    } else {
                        SAVE_BUTTON_ID
                    };
                    Some(command)
                }
                _ => None,
            };
            if let Some(command) = command {
                let _ = unsafe {
                    PostMessageW(
                        hwnd,
                        WM_COMMAND,
                        WPARAM(command as usize | ((BN_CLICKED as usize) << 16)),
                        LPARAM::default(),
                    )
                };
                continue;
            }
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    };

    let cleanup = cleanup_window(hwnd, state.as_ref());
    let creation_error = state
        .try_borrow_mut()
        .ok()
        .and_then(|mut state| state.creation_error.take());

    if matches!(
        cleanup,
        WindowCleanup::DetachFailed | WindowCleanup::UnattachedLiveWindow
    ) {
        // The window may still reference this state. Leaking the state and
        // its GDI handles is safer than freeing memory a WndProc can reach.
        let message = match cleanup {
            WindowCleanup::DetachFailed => "无法分离设置窗口状态，已停止释放设置状态".to_owned(),
            WindowCleanup::UnattachedLiveWindow => {
                "设置窗口句柄不再属于当前状态，未销毁可能是其他窗口的句柄".to_owned()
            }
            _ => unreachable!(),
        };
        let unregister = unsafe { UnregisterClassW(WINDOW_CLASS_NAME, instance) };
        std::mem::forget(state);
        return Err(match unregister {
            Ok(()) => message,
            Err(error) => format!("{message}；注销窗口类也失败：{error}"),
        });
    }

    if matches!(
        cleanup,
        WindowCleanup::Destroyed | WindowCleanup::AlreadyGone
    ) {
        delete_all_fonts(state.as_ref());
    }

    drop(state);
    let unregister = unsafe { UnregisterClassW(WINDOW_CLASS_NAME, instance) };
    match (creation_error, unregister, cleanup, loop_result) {
        (Some(error), _, _, _) => Err(error),
        (None, Err(error), _, _) => Err(format!("无法注销设置窗口类：{error}")),
        (None, Ok(()), WindowCleanup::DestroyedFailed, _) => {
            Err("设置窗口状态已分离，但销毁窗口失败".to_owned())
        }
        (None, Ok(()), _, result) => result,
    }
}

struct SettingsUiState {
    store: SettingsStore,
    settings: Settings,
    controls: SettingsControls,
    fonts: Vec<HFONT>,
    saving: bool,
    creation_error: Option<String>,
    domain_names: Vec<String>,
}

#[derive(Clone, Copy, Default)]
struct SettingsControls {
    page_edit: HWND,
    theme_combo: HWND,
    save_button: HWND,
    cancel_button: HWND,
    notice: HWND,
    labels: [HWND; 3],
    learning: HWND,
    clear_learning: HWND,
    domain_combo: HWND,
    domain_enabled: HWND,
    dictionary_status: HWND,
}

unsafe extern "system" fn settings_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Windows invokes this procedure only for a live settings
        // window whose user-data slot refers to the owner's Box.
        unsafe { handle_settings_message(hwnd, message, wparam, lparam) }
    })) {
        Ok(result) => result,
        Err(_) => {
            if message == WM_DESTROY {
                unsafe { PostQuitMessage(0) };
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

unsafe fn handle_settings_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            if lparam.0 != 0 {
                let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_CREATE => {
            let Some(state) = (unsafe { settings_state_pointer(hwnd).as_ref() }) else {
                return LRESULT(-1);
            };
            match initialize_controls(hwnd, state) {
                Ok(()) => LRESULT(0),
                Err(error) => {
                    if let Ok(mut state) = state.try_borrow_mut() {
                        state.creation_error = Some(error);
                    }
                    LRESULT(-1)
                }
            }
        }
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xffff) as i32;
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            if control_id == SAVE_BUTTON_ID && notification == BN_CLICKED {
                if let Some(state) = unsafe { settings_state_pointer(hwnd).as_ref() } {
                    save_settings(hwnd, state);
                }
            } else if control_id == CANCEL_BUTTON_ID {
                let _ = unsafe { DestroyWindow(hwnd) };
            } else if matches!(
                control_id,
                LEARNING_ID | CLEAR_LEARNING_ID | DOMAIN_ENABLED_ID
            ) && notification == BN_CLICKED
            {
                if let Some(state) = unsafe { settings_state_pointer(hwnd).as_ref() } {
                    dictionary_action(hwnd, state, control_id);
                }
            } else if control_id == DOMAIN_COMBO_ID
                && notification == windows::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE
            {
                if let Some(state) = unsafe { settings_state_pointer(hwnd).as_ref() } {
                    refresh_dictionary(hwnd, state);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut c_void);
            unsafe {
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(GetSysColor(COLOR_BTNTEXT)));
            }
            LRESULT(unsafe { GetSysColorBrush(COLOR_BTNFACE) }.0 as isize)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xffff) as u32;
            if lparam.0 != 0 {
                let suggested = unsafe { *(lparam.0 as *const RECT) };
                let _ = unsafe {
                    MoveWindow(
                        hwnd,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        TRUE,
                    )
                };
            }
            if let Some(state) = unsafe { settings_state_pointer(hwnd).as_ref() } {
                apply_layout(state, new_dpi.max(BASE_DPI));
            }
            LRESULT(0)
        }
        WM_SYSCOLORCHANGE => {
            // SAFETY: The live top-level window can repaint its system-colored
            // background after the user changes system colors.
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, TRUE);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_NCDESTROY => {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn initialize_controls(hwnd: HWND, state: &RefCell<SettingsUiState>) -> Result<(), String> {
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map(HINSTANCE::from)
        .map_err(|error| format!("无法获取设置程序模块：{error}"))?;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(BASE_DPI);
    let settings = state
        .try_borrow()
        .map(|state| state.settings.clone())
        .map_err(|_| "设置窗口状态正忙，无法初始化".to_owned())?;

    let label = create_control(
        hwnd,
        instance,
        windows::core::w!("STATIC"),
        windows::core::w!("候选页大小（1-99）"),
        WS_CHILD.0 | WS_VISIBLE.0,
        PAGE_LABEL_ID,
    )?;
    update_controls(state, |controls| controls.labels[0] = label)?;

    let page_edit = create_control(
        hwnd,
        instance,
        windows::core::w!("EDIT"),
        PCWSTR::null(),
        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
        PAGE_EDIT_ID,
    )?;
    update_controls(state, |controls| controls.page_edit = page_edit)?;

    let label = create_control(
        hwnd,
        instance,
        windows::core::w!("STATIC"),
        windows::core::w!("可输入 1 到 99。"),
        WS_CHILD.0 | WS_VISIBLE.0,
        PAGE_HINT_ID,
    )?;
    update_controls(state, |controls| controls.labels[1] = label)?;

    let label = create_control(
        hwnd,
        instance,
        windows::core::w!("STATIC"),
        windows::core::w!("候选窗主题"),
        WS_CHILD.0 | WS_VISIBLE.0,
        THEME_LABEL_ID,
    )?;
    update_controls(state, |controls| controls.labels[2] = label)?;

    let theme_combo = create_control(
        hwnd,
        instance,
        windows::core::w!("COMBOBOX"),
        PCWSTR::null(),
        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32,
        THEME_COMBO_ID,
    )?;
    update_controls(state, |controls| controls.theme_combo = theme_combo)?;

    let notice = create_control(
        hwnd,
        instance,
        windows::core::w!("STATIC"),
        windows::core::w!("保存后，请重新切换到 Flower 输入法使设置生效。"),
        WS_CHILD.0 | WS_VISIBLE.0,
        NOTICE_ID,
    )?;
    update_controls(state, |controls| controls.notice = notice)?;

    let save_button = create_control(
        hwnd,
        instance,
        windows::core::w!("BUTTON"),
        windows::core::w!("保存"),
        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32,
        SAVE_BUTTON_ID,
    )?;
    update_controls(state, |controls| controls.save_button = save_button)?;

    let cancel_button = create_control(
        hwnd,
        instance,
        windows::core::w!("BUTTON"),
        windows::core::w!("关闭"),
        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON as u32,
        CANCEL_BUTTON_ID,
    )?;
    update_controls(state, |controls| controls.cancel_button = cancel_button)?;

    for (id, class, caption, extra_style) in [
        (
            LEARNING_ID,
            windows::core::w!("BUTTON"),
            windows::core::w!("记住选词顺序（默认开启，仅保存在本机）"),
            windows::Win32::UI::WindowsAndMessaging::BS_AUTOCHECKBOX as u32,
        ),
        (
            CLEAR_LEARNING_ID,
            windows::core::w!("BUTTON"),
            windows::core::w!("清空学习记录…"),
            BS_PUSHBUTTON as u32,
        ),
        (
            DOMAIN_COMBO_ID,
            windows::core::w!("COMBOBOX"),
            PCWSTR::null(),
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
        ),
        (
            DOMAIN_ENABLED_ID,
            windows::core::w!("BUTTON"),
            windows::core::w!("启用选中专业词库（立即保存）"),
            windows::Win32::UI::WindowsAndMessaging::BS_AUTOCHECKBOX as u32,
        ),
        (
            DICTIONARY_STATUS_ID,
            windows::core::w!("STATIC"),
            PCWSTR::null(),
            0,
        ),
    ] {
        let control = create_control(
            hwnd,
            instance,
            class,
            caption,
            WS_CHILD.0
                | WS_VISIBLE.0
                | (if id == DICTIONARY_STATUS_ID {
                    0
                } else {
                    WS_TABSTOP.0
                })
                | extra_style,
            id,
        )?;
        update_controls(state, |controls| match id {
            LEARNING_ID => controls.learning = control,
            CLEAR_LEARNING_ID => controls.clear_learning = control,
            DOMAIN_COMBO_ID => controls.domain_combo = control,
            DOMAIN_ENABLED_ID => controls.domain_enabled = control,
            _ => controls.dictionary_status = control,
        })?;
    }
    refresh_dictionary(hwnd, state);

    set_control_text(
        hwnd,
        PAGE_EDIT_ID,
        &settings.candidate_page_size.to_string(),
    )?;
    for (index, label) in ["系统颜色", "浅色", "深色"].iter().enumerate() {
        let text = wide_null(label);
        let result = unsafe {
            SendMessageW(
                theme_combo,
                CB_ADDSTRING,
                WPARAM::default(),
                LPARAM(text.as_ptr() as isize),
            )
        };
        if result.0 == CB_ERR as isize {
            return Err("无法初始化主题选项".to_owned());
        }
        if index == theme_index(settings.theme) {
            unsafe {
                SendMessageW(theme_combo, CB_SETCURSEL, WPARAM(index), LPARAM::default());
            }
        }
    }

    replace_font(state, dpi).map_err(|error| format!("无法创建设置窗口字体：{error}"))?;
    layout_controls(state, dpi);
    Ok(())
}

fn update_controls(
    state: &RefCell<SettingsUiState>,
    update: impl FnOnce(&mut SettingsControls),
) -> Result<(), String> {
    let mut state = state
        .try_borrow_mut()
        .map_err(|_| "设置窗口状态正忙，无法保存控件句柄".to_owned())?;
    update(&mut state.controls);
    Ok(())
}

fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    class_name: PCWSTR,
    text: PCWSTR,
    style: u32,
    control_id: i32,
) -> Result<HWND, String> {
    let menu = HMENU(control_id as isize as *mut c_void);
    unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            text,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style),
            0,
            0,
            1,
            1,
            parent,
            menu,
            instance,
            None,
        )
    }
    .map_err(|error| format!("无法创建设置控件：{error}"))
}

fn apply_layout(state: &RefCell<SettingsUiState>, dpi: u32) {
    let _ = replace_font(state, dpi);
    layout_controls(state, dpi);
}

fn set_control_rect(
    control: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    dpi: u32,
) -> Result<(), WindowsError> {
    if control.is_invalid() {
        return Ok(());
    }
    unsafe {
        MoveWindow(
            control,
            scaled(x, dpi),
            scaled(y, dpi),
            scaled(width, dpi),
            scaled(height, dpi),
            TRUE,
        )
    }
}

fn layout_controls(state: &RefCell<SettingsUiState>, dpi: u32) {
    let Ok(controls) = state.try_borrow().map(|state| state.controls) else {
        return;
    };
    // Win32 is called only after the state borrow has ended.
    let _ = set_control_rect(controls.labels[0], 20, 20, 360, 24, dpi);
    let _ = set_control_rect(controls.page_edit, 20, 48, 90, 28, dpi);
    let _ = set_control_rect(controls.labels[1], 120, 52, 300, 24, dpi);
    let _ = set_control_rect(controls.labels[2], 20, 90, 360, 24, dpi);
    let _ = set_control_rect(controls.theme_combo, 20, 118, 180, 110, dpi);
    let _ = set_control_rect(controls.learning, 20, 168, 420, 28, dpi);
    let _ = set_control_rect(controls.clear_learning, 20, 206, 180, 30, dpi);
    let _ = set_control_rect(controls.domain_combo, 20, 254, 420, 150, dpi);
    let _ = set_control_rect(controls.domain_enabled, 20, 294, 420, 28, dpi);
    let _ = set_control_rect(controls.dictionary_status, 20, 334, 420, 120, dpi);
    let _ = set_control_rect(controls.notice, 20, 470, 420, 44, dpi);
    let _ = set_control_rect(controls.save_button, 240, 530, 90, 30, dpi);
    let _ = set_control_rect(controls.cancel_button, 350, 530, 90, 30, dpi);
}

fn replace_font(state: &RefCell<SettingsUiState>, dpi: u32) -> Result<(), WindowsError> {
    let font = create_ui_font(dpi)?;
    let controls = match state.try_borrow_mut() {
        Ok(mut state) => {
            state.fonts.push(font);
            state.controls
        }
        Err(_) => {
            unsafe {
                let _ = DeleteObject(HGDIOBJ::from(font));
            }
            return Err(WindowsError::from_hresult(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
    };
    for control in [
        controls.labels[0],
        controls.page_edit,
        controls.labels[1],
        controls.labels[2],
        controls.theme_combo,
        controls.notice,
        controls.save_button,
        controls.cancel_button,
        controls.learning,
        controls.clear_learning,
        controls.domain_combo,
        controls.domain_enabled,
        controls.dictionary_status,
    ] {
        if !control.is_invalid() {
            unsafe {
                SendMessageW(
                    control,
                    windows::Win32::UI::WindowsAndMessaging::WM_SETFONT,
                    WPARAM(font.0 as usize),
                    LPARAM(TRUE.0 as isize),
                );
            }
        }
    }
    Ok(())
}

fn create_ui_font(dpi: u32) -> Result<HFONT, WindowsError> {
    let height = -scaled(16, dpi);
    let font = unsafe {
        CreateFontW(
            height,
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
        Err(WindowsError::from_win32())
    } else {
        Ok(font)
    }
}

fn delete_all_fonts(state: &RefCell<SettingsUiState>) {
    let fonts = state
        .try_borrow_mut()
        .ok()
        .map(|mut state| std::mem::take(&mut state.fonts))
        .unwrap_or_default();
    for font in fonts {
        if font.is_invalid() {
            continue;
        }
        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(font));
        }
    }
}

fn theme_dropdown_open(state: &RefCell<SettingsUiState>) -> bool {
    let Ok(controls) = state.try_borrow().map(|state| state.controls) else {
        return false;
    };
    [controls.theme_combo, controls.domain_combo]
        .iter()
        .any(|&combo| {
            !combo.is_invalid()
                && unsafe {
                    SendMessageW(
                        combo,
                        CB_GETDROPPEDSTATE,
                        WPARAM::default(),
                        LPARAM::default(),
                    )
                }
                .0 != 0
        })
}

fn selected_domain(state: &RefCell<SettingsUiState>, combo: HWND) -> Option<String> {
    let selected = unsafe {
        SendMessageW(
            combo,
            windows::Win32::UI::WindowsAndMessaging::CB_GETCURSEL,
            WPARAM::default(),
            LPARAM::default(),
        )
    }
    .0;
    usize::try_from(selected)
        .ok()
        .and_then(|index| state.try_borrow().ok()?.domain_names.get(index).cloned())
}

fn set_checked(control: HWND, checked: bool) {
    unsafe {
        SendMessageW(
            control,
            windows::Win32::UI::WindowsAndMessaging::BM_SETCHECK,
            WPARAM(usize::from(checked)),
            LPARAM::default(),
        );
    }
}

fn is_checked(control: HWND) -> bool {
    unsafe {
        SendMessageW(
            control,
            windows::Win32::UI::WindowsAndMessaging::BM_GETCHECK,
            WPARAM::default(),
            LPARAM::default(),
        )
    }
    .0 == 1
}

/// Re-read rather than treating a failed load as an empty/default dictionary.
/// No RefCell borrow is held across a Win32 call (which can re-enter WndProc).
fn refresh_dictionary(hwnd: HWND, state: &RefCell<SettingsUiState>) {
    let Ok(controls) = state.try_borrow().map(|state| state.controls) else {
        return;
    };
    let selected = selected_domain(state, controls.domain_combo);
    let result = DictionaryStore::for_current_user().and_then(|store| store.load());
    match result {
        Ok(data) => {
            let names: Vec<_> = data.domains.iter().map(|item| item.name.clone()).collect();
            let selected_index = selected
                .as_ref()
                .and_then(|name| names.iter().position(|item| item == name))
                .unwrap_or(0);
            if let Ok(mut state) = state.try_borrow_mut() {
                state.domain_names = names;
            }
            unsafe {
                SendMessageW(
                    controls.domain_combo,
                    windows::Win32::UI::WindowsAndMessaging::CB_RESETCONTENT,
                    WPARAM::default(),
                    LPARAM::default(),
                );
            }
            for item in &data.domains {
                let caption = wide_null(&format!("{}（{} 条）", item.name, item.entries.len()));
                unsafe {
                    SendMessageW(
                        controls.domain_combo,
                        CB_ADDSTRING,
                        WPARAM::default(),
                        LPARAM(caption.as_ptr() as isize),
                    );
                }
            }
            unsafe {
                SendMessageW(
                    controls.domain_combo,
                    CB_SETCURSEL,
                    WPARAM(selected_index),
                    LPARAM::default(),
                );
            }
            set_checked(controls.learning, data.adaptive_ranking_enabled);
            set_checked(
                controls.domain_enabled,
                data.domains
                    .get(selected_index)
                    .is_some_and(|item| item.enabled),
            );
            let status = format!(
                "个人词条：{}；排序作用域：{}；专业词库：{}。\n选中词库：{}。\n成功选词前移一位；设置保存后重新切换输入法生效。\n清空学习不会删除个人词条。",
                data.entries.len(), data.rankings.len(), data.domains.len(),
                data.domains.get(selected_index).map(|item| if item.enabled { "已启用" } else { "已停用" }).unwrap_or("无"));
            let _ = set_control_text(hwnd, DICTIONARY_STATUS_ID, &status);
        }
        Err(error) => {
            set_checked(controls.learning, false);
            set_checked(controls.domain_enabled, false);
            let _ = set_control_text(hwnd, DICTIONARY_STATUS_ID,
                &format!("用户词典读取失败：{error}\n未重建或覆盖文件；词典操作不可用。请修复文件后重新打开设置。"));
        }
    }
}

fn dictionary_action(hwnd: HWND, state: &RefCell<SettingsUiState>, id: i32) {
    let Some(_guard) = SaveGuard::acquire(state) else {
        return;
    };
    let Ok(controls) = state.try_borrow().map(|state| state.controls) else {
        return;
    };
    // Capture the explicit checkbox action before refreshing from disk.
    let checked = is_checked(if id == LEARNING_ID {
        controls.learning
    } else {
        controls.domain_enabled
    });
    let name = selected_domain(state, controls.domain_combo);
    if id == CLEAR_LEARNING_ID
        && (message_box(
            hwnd,
            "确定清空全部本地学习记录？\n\n不会删除个人词条或专业词库。此操作不可撤销。",
            "Flower 设置",
            MB_YESNO,
        ) != IDYES
            || !window_is_owned(hwnd, state))
    {
        return;
    }
    let result = (|| -> Result<(), String> {
        let store = DictionaryStore::for_current_user().map_err(|e| e.to_string())?;
        // Fail closed on corruption. The store also validates again under its write lock.
        store.load().map_err(|e| e.to_string())?;
        match id {
            LEARNING_ID => store.set_learning_enabled(checked),
            CLEAR_LEARNING_ID => store.clear_learning(),
            DOMAIN_ENABLED_ID => store.set_domain_enabled(
                name.as_deref()
                    .ok_or_else(|| "请先导入并选择一个专业词库。".to_owned())?,
                checked,
            ),
            _ => return Ok(()),
        }
        .map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        message_box(
            hwnd,
            &format!("词典操作失败：{error}\n未使用默认词典覆盖文件。"),
            "Flower 设置",
            MB_OK | MB_ICONERROR,
        );
    }
    if window_is_owned(hwnd, state) {
        refresh_dictionary(hwnd, state);
    }
}

fn save_settings(hwnd: HWND, state: &RefCell<SettingsUiState>) {
    let Some(_saving_guard) = SaveGuard::acquire(state) else {
        return;
    };

    let settings = match read_form(state) {
        Ok(settings) => settings,
        Err(error) => {
            message_box(hwnd, &error, "Flower 设置", MB_OK | MB_ICONERROR);
            if let Ok(page_edit) = state.try_borrow().map(|state| state.controls.page_edit) {
                if !page_edit.is_invalid() && window_is_owned(hwnd, state) {
                    let _ = unsafe { SetFocus(page_edit) };
                }
            }
            return;
        }
    };

    let Some(store) = state.try_borrow().ok().map(|state| state.store.clone()) else {
        return;
    };
    if let Err(error) = store.save(&settings) {
        let message = format!(
            "保存设置失败：{error}\n\n路径：{}\n\n原设置文件未被替换，\
             请检查目录权限后重试。",
            store.path().display()
        );
        message_box(hwnd, &message, "Flower 设置", MB_OK | MB_ICONERROR);
        return;
    }

    if let Ok(mut state) = state.try_borrow_mut() {
        state.settings = settings;
    }
    if window_is_owned(hwnd, state) {
        message_box(
            hwnd,
            "设置已保存。\n\n请重新切换到 Flower 输入法后生效。",
            "Flower 设置",
            MB_OK,
        );
        // The confirmation dialog pumped messages. Re-check ownership before
        // destroying a handle that may no longer belong to this window.
        if window_is_owned(hwnd, state) {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }
}

fn read_form(state: &RefCell<SettingsUiState>) -> Result<Settings, String> {
    let controls = state
        .try_borrow()
        .map(|state| state.controls)
        .map_err(|_| "设置窗口状态正忙，无法读取设置".to_owned())?;
    let page_text = read_control_text(controls.page_edit)?;
    let page_size = parse_candidate_page_size_input(&page_text).ok_or_else(|| {
        format!(
            "候选页大小必须是 {MIN_CANDIDATE_PAGE_SIZE} 到 {MAX_CANDIDATE_PAGE_SIZE} 之间的整数。"
        )
    })?;
    let selected = unsafe {
        SendMessageW(
            controls.theme_combo,
            windows::Win32::UI::WindowsAndMessaging::CB_GETCURSEL,
            WPARAM::default(),
            LPARAM::default(),
        )
    }
    .0;
    let theme = match selected {
        0 => Theme::System,
        1 => Theme::Light,
        2 => Theme::Dark,
        _ => return Err("请选择候选窗主题。".to_owned()),
    };
    let settings = Settings {
        candidate_page_size: page_size,
        theme,
    };
    settings
        .validate()
        .map_err(|error| format!("设置值无效：{error}"))?;
    Ok(settings)
}

fn read_control_text(control: HWND) -> Result<String, String> {
    if control.is_invalid() {
        return Err("无法读取候选页大小输入框。".to_owned());
    }
    let length = unsafe { GetWindowTextLengthW(control) };
    if length <= 0 {
        return Err(format!(
            "请输入 {MIN_CANDIDATE_PAGE_SIZE} 到 {MAX_CANDIDATE_PAGE_SIZE} 之间的候选页大小。"
        ));
    }
    if length > 64 {
        return Err("候选页大小输入过长，请输入 1 到 99 之间的整数。".to_owned());
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(control, &mut buffer) };
    if copied <= 0 {
        return Err(format!(
            "请输入 {MIN_CANDIDATE_PAGE_SIZE} 到 {MAX_CANDIDATE_PAGE_SIZE} 之间的候选页大小。"
        ));
    }
    buffer.truncate(copied as usize);
    String::from_utf16(&buffer).map_err(|_| "候选页大小包含无效字符。".to_owned())
}

struct SaveGuard<'a> {
    state: &'a RefCell<SettingsUiState>,
}

impl<'a> SaveGuard<'a> {
    fn acquire(state: &'a RefCell<SettingsUiState>) -> Option<Self> {
        let mut current = state.try_borrow_mut().ok()?;
        if current.saving {
            None
        } else {
            current.saving = true;
            Some(Self { state })
        }
    }
}

impl Drop for SaveGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.saving = false;
        }
    }
}

fn theme_index(theme: Theme) -> usize {
    match theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    }
}

fn set_control_text(hwnd: HWND, control_id: i32, text: &str) -> Result<(), String> {
    let text = wide_null(text);
    unsafe { SetDlgItemTextW(hwnd, control_id, PCWSTR::from_raw(text.as_ptr())) }
        .map_err(|error| format!("无法设置控件文本：{error}"))
}

fn message_box(
    owner: HWND,
    message: &str,
    caption: &str,
    style: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    let message = wide_null(message);
    let caption = wide_null(caption);
    unsafe {
        MessageBoxW(
            owner,
            PCWSTR::from_raw(message.as_ptr()),
            PCWSTR::from_raw(caption.as_ptr()),
            style,
        )
    }
}

fn state_pointer(state: &RefCell<SettingsUiState>) -> *const c_void {
    (state as *const RefCell<SettingsUiState>).cast::<c_void>()
}

fn settings_state_pointer(hwnd: HWND) -> *mut RefCell<SettingsUiState> {
    unsafe {
        GetWindowLongPtrW(hwnd, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA)
            as *mut RefCell<SettingsUiState>
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowCleanup {
    AlreadyGone,
    Destroyed,
    DestroyedFailed,
    DetachFailed,
    UnattachedLiveWindow,
}

fn window_is_owned(hwnd: HWND, state: &RefCell<SettingsUiState>) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    if !unsafe { IsWindow(hwnd) }.as_bool() {
        return false;
    }
    let expected = state_pointer(state) as isize;
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) == expected }
}

fn cleanup_window(hwnd: HWND, state: &RefCell<SettingsUiState>) -> WindowCleanup {
    if hwnd.is_invalid() {
        return WindowCleanup::AlreadyGone;
    }
    if !unsafe { IsWindow(hwnd) }.as_bool() {
        return WindowCleanup::AlreadyGone;
    }

    let expected = state_pointer(state) as isize;
    let current = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if current != expected {
        return WindowCleanup::UnattachedLiveWindow;
    }

    let previous = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
    if previous != expected {
        return WindowCleanup::DetachFailed;
    }

    match unsafe { DestroyWindow(hwnd) } {
        Ok(()) => WindowCleanup::Destroyed,
        Err(_) => WindowCleanup::DestroyedFailed,
    }
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn scaled(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { BASE_DPI } else { dpi.min(480) };
    ((i64::from(value) * i64::from(dpi) + i64::from(BASE_DPI / 2)) / i64::from(BASE_DPI)) as i32
}
