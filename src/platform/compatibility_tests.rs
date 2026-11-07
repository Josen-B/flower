#[implement(ITfEditSession)]
struct CustomCompatibilityProbe {
    context: ITfContext,
    expected: ScopeBlockReason,
}

impl ITfEditSession_Impl for CustomCompatibilityProbe_Impl {
    fn DoEditSession(&self, cookie: u32) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let window = COMPAT_VIEW.with(Cell::get);
        assert_eq!(
            check_ai_scope(&self.context, cookie, false),
            Err(self.expected)
        );
        let eligible = matches!(
            self.expected,
            ScopeBlockReason::Undeclared | ScopeBlockReason::Empty
        );
        assert_eq!(
            check_ai_scope(&self.context, cookie, true).is_ok(),
            eligible
        );
        assert!(!scope_allows_learning(&self.context, cookie));
        if eligible {
            unsafe {
                let original = GetWindowLongW(window, GWL_STYLE);
                for bits in [0x20, 0x800, WS_DISABLED.0] {
                    SetWindowLongW(window, GWL_STYLE, original | bits as i32);
                    assert!(check_ai_scope(&self.context, cookie, true).is_err());
                    SetWindowLongW(window, GWL_STYLE, original);
                }
                // A successful send gate must not grant a later publication.
                COMPAT_VIEW.with(|v| v.set(Default::default()));
                assert!(check_ai_scope(&self.context, cookie, true).is_err());
                COMPAT_VIEW.with(|v| v.set(window));
                assert!(check_ai_scope(&self.context, cookie, true).is_ok());
                assert!(!checked_edit_window(
                    window,
                    || {
                        SetWindowLongW(window, GWL_STYLE, original | 0x800);
                        Some(window)
                    },
                    true
                ));
                SetWindowLongW(window, GWL_STYLE, original);
            }
        }
        Ok(())
    }
}

#[test]
fn native_custom_compatibility_read_cookie_matrix() {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .unwrap();
    let _apartment = Apartment;
    unsafe {
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("Static"),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            100,
            30,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let previous = GetFocus();
        let _ = SetFocus(window);
        COMPAT_VIEW.with(|v| v.set(window));
        let mut cases = vec![(VARIANT::default(), ScopeBlockReason::Undeclared)];
        for (scopes, reason) in [
            (vec![], ScopeBlockReason::Empty),
            (vec![31], ScopeBlockReason::Sensitive),
            (vec![61], ScopeBlockReason::Sensitive),
            (vec![63], ScopeBlockReason::Sensitive),
            (vec![999], ScopeBlockReason::Unsupported),
        ] {
            let scope: IUnknown = TestScope(scopes).into();
            cases.push((VARIANT::from(scope), reason));
        }
        cases.push((VARIANT::from(42i32), ScopeBlockReason::InvalidValue));
        let unknown: IUnknown = ClassFactory::new().into();
        cases.push((VARIANT::from(unknown), ScopeBlockReason::Interface));
        for (value, expected) in cases {
            let owner: IUnknown = ScopeOwner {
                value: RefCell::new(value),
                calls: Rc::new(Cell::new(0)),
                fail: false,
            }
            .into();
            let host = NativeDocument::with_owner(Some(&owner)).unwrap();
            let session: ITfEditSession = CustomCompatibilityProbe {
                context: host.context.clone(),
                expected,
            }
            .into();
            host.context
                .RequestEditSession(host.client, &session, TF_ES_SYNC | TF_ES_READ)
                .unwrap()
                .ok()
                .unwrap();
        }
        COMPAT_VIEW.with(|v| v.set(Default::default()));
        let _ = SetFocus(previous);
        DestroyWindow(window).unwrap();
    }
}

#[test]
fn native_custom_compatibility_publication_rechecks_without_learning() {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .unwrap();
    let _apartment = Apartment;
    unsafe {
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("Static"),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            100,
            30,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let previous = GetFocus();
        let _ = SetFocus(window);
        COMPAT_VIEW.with(|v| v.set(window));
        for change in 0..4 {
            let calls = Rc::new(Cell::new(0));
            let owner: IUnknown = ScopeOwner {
                value: RefCell::new(VARIANT::default()),
                calls: calls.clone(),
                fail: false,
            }
            .into();
            let host = NativeDocument::with_owner(Some(&owner)).unwrap();
            let capture = Rc::new(CapturedAi::default());
            let (service, shared) = ai_fixture(&host, &capture, &calls);
            tick_ai(&shared);
            pump_ai(&shared);
            assert_eq!(capture.requests.borrow().len(), 1);
            match change {
                1 => {
                    let scope: IUnknown = TestScope(vec![31]).into();
                    let interface: ITfContextOwner = owner.cast().unwrap();
                    let implementation =
                        <ITfContextOwner as AsImpl<ScopeOwner>>::as_impl(&interface);
                    *implementation.value.borrow_mut() = VARIANT::from(scope);
                }
                2 => {
                    let _ = SetFocus(windows::Win32::Foundation::HWND::default());
                }
                3 => {
                    shared
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .settings
                        .ai_undeclared_scope_compatibility = false;
                }
                _ => {}
            }
            captured_reply(&capture);
            tick_ai(&shared);
            pump_ai(&shared);
            assert_eq!(
                ai_status(shared.borrow().as_ref().unwrap()) == AiStatus::Suggested,
                change == 0
            );
            if change == 0 {
                let sink: ITfKeyEventSink = service.cast().unwrap();
                send_key(&sink, &host.context, b'2');
                let state = shared.borrow();
                let active = state.as_ref().unwrap();
                assert!(active.rankings.is_empty());
                assert!(active
                    .contexts
                    .get(&context_key(&host.context))
                    .unwrap()
                    .router
                    .pending_promotions()
                    .is_empty());
            }
            drop(shared.borrow_mut().take());
            drop(service);
            let _ = SetFocus(window);
        }
        COMPAT_VIEW.with(|v| v.set(Default::default()));
        let _ = SetFocus(previous);
        DestroyWindow(window).unwrap();
    }
}
