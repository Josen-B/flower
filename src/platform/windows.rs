#![cfg_attr(not(target_os = "windows"), forbid(unsafe_code))]
//! Windows Text Services Framework boundary for Flower.
//!
//! The TSF/COM module is compiled only for Windows. It exports the in-process
//! server entry points, a class factory for the Flower text service, and a
//! text service that maintains per-context engine sessions and applies
//! composition updates through synchronous TSF edit sessions.

pub const TEXT_SERVICE_DESCRIPTION: &str = "Flower Pinyin input method";

pub fn is_supported_platform() -> bool {
    cfg!(target_os = "windows")
}

#[cfg(target_os = "windows")]
pub const FLOWER_TEXT_SERVICE_CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(0x3d5b7cb1_96cb_4f44_a792_64304f6e7426);

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod windows_exports {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard};

    use crate::engine::{
        map_windows_virtual_key, CompositionUpdate, EngineRouter, KeyEvent, Modifiers,
    };
    use crate::platform::composition::{CompositionController, CompositionHost};
    use windows::core::{implement, AsImpl, Error, IUnknown, Interface, Result, GUID, HRESULT};
    use windows::Win32::Foundation::{
        BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER,
        E_UNEXPECTED, FALSE, LPARAM, S_FALSE, S_OK, TRUE, WPARAM,
    };
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::TextServices::{
        ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfEditSession,
        ITfEditSession_Impl, ITfInsertAtSelection, ITfKeyEventSink, ITfKeyEventSink_Impl,
        ITfKeystrokeMgr, ITfRange, ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfThreadMgr,
        TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY,
    };
    use windows_core::{ComObjectInterface, IUnknownImpl};

    use super::FLOWER_TEXT_SERVICE_CLSID;

    static OBJECT_COUNT: AtomicU32 = AtomicU32::new(0);
    static SERVER_LOCK_COUNT: AtomicU32 = AtomicU32::new(0);

    struct ObjectRegistration;

    impl ObjectRegistration {
        fn new() -> Self {
            OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for ObjectRegistration {
        fn drop(&mut self) {
            let _ = OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn release_server_lock() -> Result<()> {
        let mut current = SERVER_LOCK_COUNT.load(Ordering::SeqCst);

        while current > 0 {
            match SERVER_LOCK_COUNT.compare_exchange_weak(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }

        Err(E_UNEXPECTED.into())
    }

    fn can_unload() -> bool {
        OBJECT_COUNT.load(Ordering::SeqCst) == 0 && SERVER_LOCK_COUNT.load(Ordering::SeqCst) == 0
    }

    #[implement(IClassFactory)]
    struct ClassFactory {
        _registration: ObjectRegistration,
    }

    impl ClassFactory {
        fn new() -> Self {
            Self {
                _registration: ObjectRegistration::new(),
            }
        }
    }

    impl IClassFactory_Impl for ClassFactory_Impl {
        fn CreateInstance(
            &self,
            outer: Option<&IUnknown>,
            interface_id: *const GUID,
            object: *mut *mut c_void,
        ) -> Result<()> {
            if object.is_null() || interface_id.is_null() {
                return Err(E_POINTER.into());
            }

            // SAFETY: The null check above establishes that the caller supplied
            // the required output pointer.
            unsafe {
                object.write(ptr::null_mut());
            }

            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }

            let service: IUnknown = TextService::new().into();

            // SAFETY: `interface_id` and `object` are non-null output pointers,
            // and `service` is a live COM identity. The query adds a reference
            // for the caller; dropping `service` releases only our local one.
            unsafe {
                service.query(interface_id, object).ok()?;
            }

            Ok(())
        }

        fn LockServer(&self, lock: BOOL) -> Result<()> {
            if lock.as_bool() {
                SERVER_LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
                Ok(())
            } else {
                release_server_lock()
            }
        }
    }

    type ContextKey = usize;

    #[derive(Clone)]
    struct ContextEntry {
        context: ITfContext,
        router: EngineRouter,
        composition: Option<ITfComposition>,
        controller: CompositionController,
    }

    impl ContextEntry {
        fn new(context: &ITfContext) -> Option<Self> {
            let router = EngineRouter::new().ok()?;
            Some(Self {
                context: context.clone(),
                router,
                composition: None,
                controller: CompositionController::new(),
            })
        }
    }

    struct Activation {
        keystroke_manager: ITfKeystrokeMgr,
        client_id: u32,
        contexts: HashMap<ContextKey, ContextEntry>,
    }

    fn context_key(context: &ITfContext) -> ContextKey {
        // TSF supplies the same COM interface identity across callbacks for a
        // context. The raw interface pointer is stable; a Rust reference
        // address is not.
        context.as_raw() as usize
    }

    fn virtual_key_is_down(virtual_key: VIRTUAL_KEY) -> bool {
        // SAFETY: GetKeyState has no pointer arguments and reports the current
        // thread's keyboard state.
        unsafe { GetKeyState(virtual_key.0 as i32) < 0 }
    }

    fn map_key_event(wparam: WPARAM) -> Option<KeyEvent> {
        let modifiers = Modifiers {
            shift: virtual_key_is_down(VK_SHIFT),
            control: virtual_key_is_down(VK_CONTROL),
            alt: virtual_key_is_down(VK_MENU),
            meta: virtual_key_is_down(VK_LWIN) || virtual_key_is_down(VK_RWIN),
        };

        if modifiers.is_command() {
            return None;
        }

        map_windows_virtual_key(wparam.0 as u32, modifiers)
    }

    struct TfCompositionHost {
        context: ITfContext,
        composition: Option<ITfComposition>,
        edit_cookie: u32,
    }

    impl TfCompositionHost {
        fn new(context: ITfContext, composition: Option<ITfComposition>, edit_cookie: u32) -> Self {
            Self {
                context,
                composition,
                edit_cookie,
            }
        }

        fn into_composition(self) -> Option<ITfComposition> {
            self.composition
        }

        fn text_range(&mut self) -> Result<ITfRange> {
            let Some(composition) = self.composition.clone() else {
                return Err(E_UNEXPECTED.into());
            };

            // SAFETY: The composition is a live TSF object for this context.
            match unsafe { composition.GetRange() } {
                Ok(range) => Ok(range),
                Err(error) => {
                    self.end_composition_best_effort();
                    Err(error)
                }
            }
        }

        fn end_composition(&mut self) -> Result<()> {
            let Some(composition) = self.composition.take() else {
                return Ok(());
            };

            // SAFETY: The composition is live and this edit session owns a
            // read/write lock through `edit_cookie`.
            if let Err(error) = unsafe { composition.EndComposition(self.edit_cookie) } {
                self.composition = Some(composition);
                return Err(error);
            }

            Ok(())
        }

        fn end_composition_best_effort(&mut self) {
            let Some(composition) = self.composition.take() else {
                return;
            };

            // SAFETY: The composition was live before cleanup began. Cleanup is
            // best effort, and the original failure remains the reported error.
            let _ = unsafe { composition.EndComposition(self.edit_cookie) };
        }
    }

    impl CompositionHost for TfCompositionHost {
        type Error = Error;

        fn begin(&mut self, preedit: &str) -> Result<()> {
            let insert: ITfInsertAtSelection = self.context.cast()?;
            // SAFETY: A query-only insertion returns a live range at the
            // selection without changing document text.
            let range =
                unsafe { insert.InsertTextAtSelection(self.edit_cookie, TF_IAS_QUERYONLY, &[]) }?;
            let composition_manager: ITfContextComposition = self.context.cast()?;

            // SAFETY: The queried range and context are live in the current
            // read/write edit session.
            let composition = unsafe {
                composition_manager.StartComposition(
                    self.edit_cookie,
                    &range,
                    None::<&ITfCompositionSink>,
                )
            }?;
            self.composition = Some(composition.clone());

            let text: Vec<u16> = preedit.encode_utf16().collect();
            // SAFETY: The new composition belongs to this edit session and the
            // UTF-16 buffer remains alive for the duration of the call.
            let set_text = unsafe { composition.GetRange() }.and_then(|range| {
                // SAFETY: `range` is live and `text` is a valid UTF-16 buffer.
                unsafe { range.SetText(self.edit_cookie, 0, &text) }
            });

            if let Err(error) = set_text {
                self.end_composition_best_effort();
                return Err(error);
            }

            Ok(())
        }

        fn replace(&mut self, preedit: &str) -> Result<()> {
            let range = self.text_range()?;
            let text: Vec<u16> = preedit.encode_utf16().collect();
            // SAFETY: The range belongs to the active composition, and the
            // UTF-16 buffer remains alive for the duration of the call.
            unsafe { range.SetText(self.edit_cookie, 0, &text) }
        }

        fn commit(&mut self, text: &str) -> Result<()> {
            let range = self.text_range()?;
            let text: Vec<u16> = text.encode_utf16().collect();
            // SAFETY: The range belongs to the active composition, and the
            // UTF-16 buffer remains alive for the duration of the call.
            unsafe { range.SetText(self.edit_cookie, 0, &text) }?;
            self.end_composition()
        }

        fn cancel(&mut self) -> Result<()> {
            self.end_composition()
        }
    }

    struct EditSessionOutcome {
        composition: Option<ITfComposition>,
        controller: CompositionController,
    }

    struct EditSessionState {
        composition: Option<ITfComposition>,
        controller: CompositionController,
        outcome: Option<EditSessionOutcome>,
    }

    #[implement(ITfEditSession)]
    struct EditSession {
        context: ITfContext,
        update: CompositionUpdate,
        state: Mutex<EditSessionState>,
    }

    impl EditSession {
        fn new(
            context: ITfContext,
            update: CompositionUpdate,
            composition: Option<ITfComposition>,
            controller: CompositionController,
        ) -> Self {
            Self {
                context,
                update,
                state: Mutex::new(EditSessionState {
                    composition,
                    controller,
                    outcome: None,
                }),
            }
        }

        fn lock_state(&self) -> MutexGuard<'_, EditSessionState> {
            match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            }
        }

        fn take_outcome(&self) -> Option<EditSessionOutcome> {
            let mut state = self.lock_state();
            state.outcome.take()
        }
    }

    impl ITfEditSession_Impl for EditSession_Impl {
        fn DoEditSession(&self, edit_cookie: u32) -> Result<()> {
            let this = self.get_impl();
            let (composition, controller) = {
                let state = this.lock_state();
                (state.composition.clone(), state.controller)
            };
            let mut host = TfCompositionHost::new(this.context.clone(), composition, edit_cookie);
            let mut controller = controller;
            let result = controller.apply_update(&mut host, &this.update);
            let composition = host.into_composition();

            let mut state = this.lock_state();
            state.composition = composition.clone();
            if result.is_ok() {
                state.controller = controller;
                state.outcome = Some(EditSessionOutcome {
                    composition,
                    controller,
                });
            }

            result
        }
    }

    #[implement(ITfTextInputProcessor, ITfKeyEventSink)]
    struct TextService {
        activation: Mutex<Option<Activation>>,
        _registration: ObjectRegistration,
    }

    impl TextService {
        fn new() -> Self {
            Self {
                activation: Mutex::new(None),
                _registration: ObjectRegistration::new(),
            }
        }

        fn lock_activation(&self) -> MutexGuard<'_, Option<Activation>> {
            match self.activation.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            }
        }

        fn publish_context(&self, key: ContextKey, entry: ContextEntry) {
            let mut entry = Some(entry);
            let displaced = {
                let mut activation = self.lock_activation();
                activation.as_mut().and_then(|active| {
                    let entry = entry.take()?;
                    Some(active.contexts.insert(key, entry))
                })
            };

            drop(entry);
            drop(displaced);
        }
    }

    impl ITfTextInputProcessor_Impl for TextService_Impl {
        fn Activate(&self, thread_manager: Option<&ITfThreadMgr>, client_id: u32) -> Result<()> {
            let Some(thread_manager) = thread_manager else {
                return Err(E_INVALIDARG.into());
            };

            let keystroke_manager: ITfKeystrokeMgr = thread_manager.cast()?;
            let key_sink: ITfKeyEventSink =
                <Self as ComObjectInterface<ITfKeyEventSink>>::as_interface_ref(self).to_owned();

            {
                let mut activation = self.get_impl().lock_activation();
                if activation.is_some() {
                    return Err(E_UNEXPECTED.into());
                }

                *activation = Some(Activation {
                    keystroke_manager: keystroke_manager.clone(),
                    client_id,
                    contexts: HashMap::new(),
                });
            }

            // SAFETY: Both interfaces belong to live COM objects. TSF retains
            // its own reference to `key_sink` after a successful advise.
            let advised =
                unsafe { keystroke_manager.AdviseKeyEventSink(client_id, &key_sink, TRUE) };

            if let Err(error) = advised {
                let stale = {
                    let mut activation = self.get_impl().lock_activation();
                    let should_rollback = matches!(
                        activation.as_ref(),
                        Some(active) if active.client_id == client_id
                    );
                    if should_rollback {
                        activation.take()
                    } else {
                        None
                    }
                };
                drop(stale);
                return Err(error);
            }

            Ok(())
        }

        fn Deactivate(&self) -> Result<()> {
            let active = {
                let mut activation = self.get_impl().lock_activation();
                activation.take()
            };
            let Some(mut active) = active else {
                return Ok(());
            };
            active.contexts.clear();

            // SAFETY: The keystroke manager was obtained from the thread manager
            // during activation and remains live in this state.
            let unadvised = unsafe {
                active
                    .keystroke_manager
                    .UnadviseKeyEventSink(active.client_id)
            };
            if let Err(error) = unadvised {
                let displaced = {
                    let mut activation = self.get_impl().lock_activation();
                    if activation.is_none() {
                        activation.replace(active)
                    } else {
                        Some(active)
                    }
                };
                drop(displaced);
                return Err(error);
            }

            drop(active);
            Ok(())
        }
    }

    impl ITfKeyEventSink_Impl for TextService_Impl {
        fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
            Ok(())
        }

        fn OnTestKeyDown(
            &self,
            context: Option<&ITfContext>,
            wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            let Some(context) = context else {
                return Ok(FALSE);
            };
            let Some(event) = map_key_event(wparam) else {
                return Ok(FALSE);
            };
            let key = context_key(context);

            let router = {
                let activation = self.get_impl().lock_activation();
                let Some(active) = activation.as_ref() else {
                    return Ok(FALSE);
                };
                active.contexts.get(&key).map(|entry| entry.router.clone())
            };
            let router = match router {
                Some(router) => router,
                None => match EngineRouter::new() {
                    Ok(router) => router,
                    Err(_) => return Ok(FALSE),
                },
            };

            match router.test_key(event) {
                Ok(true) => Ok(TRUE),
                Ok(false) | Err(_) => Ok(FALSE),
            }
        }

        fn OnTestKeyUp(
            &self,
            _context: Option<&ITfContext>,
            _wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            Ok(FALSE)
        }

        fn OnKeyDown(
            &self,
            context: Option<&ITfContext>,
            wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            let Some(context) = context else {
                return Ok(FALSE);
            };
            let Some(event) = map_key_event(wparam) else {
                return Ok(FALSE);
            };
            let key = context_key(context);

            let (entry, client_id) = {
                let mut activation = self.get_impl().lock_activation();
                let Some(active) = activation.as_mut() else {
                    return Ok(FALSE);
                };
                (active.contexts.remove(&key), active.client_id)
            };

            let was_existing = entry.is_some();
            let old_entry = entry.clone();
            let mut entry = match entry {
                Some(entry) => entry,
                None => match ContextEntry::new(context) {
                    Some(entry) => entry,
                    None => return Ok(FALSE),
                },
            };

            let update = match entry.router.process_key(event) {
                Ok(update) => update,
                Err(_) => {
                    if let Some(old_entry) = old_entry {
                        self.get_impl().publish_context(key, old_entry);
                    }
                    return Ok(FALSE);
                }
            };

            if !update.handled {
                if was_existing {
                    self.get_impl().publish_context(key, entry);
                }
                return Ok(FALSE);
            }

            let session: ITfEditSession = EditSession::new(
                entry.context.clone(),
                update,
                entry.composition.take(),
                entry.controller,
            )
            .into();

            // SAFETY: The context is live and the edit session is kept alive
            // until the synchronous request returns.
            let requested = unsafe {
                entry
                    .context
                    .RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READWRITE)
            };
            let outcome = match requested {
                // SAFETY: `session` was created from `EditSession` above.
                Ok(result) if result.is_ok() => unsafe {
                    <ITfEditSession as AsImpl<EditSession>>::as_impl(&session).take_outcome()
                },
                Ok(_) | Err(_) => None,
            };

            match outcome {
                Some(outcome) => {
                    entry.composition = outcome.composition;
                    entry.controller = outcome.controller;
                    self.get_impl().publish_context(key, entry);
                    Ok(TRUE)
                }
                None => {
                    if let Some(old_entry) = old_entry {
                        self.get_impl().publish_context(key, old_entry);
                    }
                    Ok(FALSE)
                }
            }
        }

        fn OnKeyUp(
            &self,
            _context: Option<&ITfContext>,
            _wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            Ok(FALSE)
        }

        fn OnPreservedKey(
            &self,
            _context: Option<&ITfContext>,
            _guid: *const GUID,
        ) -> Result<BOOL> {
            Ok(FALSE)
        }
    }

    fn get_class_object(
        class_id: *const GUID,
        interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() {
            return Err(E_POINTER.into());
        }

        // SAFETY: The null check above establishes that the COM output pointer
        // is writable.
        unsafe {
            object.write(ptr::null_mut());
        }

        if class_id.is_null() || interface_id.is_null() {
            return Err(E_POINTER.into());
        }

        // SAFETY: Both identifiers were checked for null above.
        if unsafe { *class_id } != FLOWER_TEXT_SERVICE_CLSID {
            return Err(CLASS_E_CLASSNOTAVAILABLE.into());
        }

        let factory: IUnknown = ClassFactory::new().into();

        // SAFETY: `interface_id` and `object` are non-null. The query transfers
        // one counted interface reference to the caller.
        unsafe {
            factory.query(interface_id, object).ok()?;
        }

        Ok(())
    }

    /// Reports whether the DLL has no live COM objects or server locks.
    #[no_mangle]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        if can_unload() {
            S_OK
        } else {
            S_FALSE
        }
    }

    /// Returns the Flower class factory for the requested COM interface.
    #[no_mangle]
    pub extern "system" fn DllGetClassObject(
        class_id: *const GUID,
        interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            get_class_object(class_id, interface_id, object)
        })) {
            Ok(Ok(())) => S_OK,
            Ok(Err(error)) => error.code(),
            Err(_) => E_UNEXPECTED,
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_exports::{DllCanUnloadNow, DllGetClassObject};

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{FLOWER_TEXT_SERVICE_CLSID, TEXT_SERVICE_DESCRIPTION};

    #[test]
    fn text_service_clsid_matches_registration() {
        assert_eq!(
            FLOWER_TEXT_SERVICE_CLSID.to_u128(),
            0x3d5b7cb1_96cb_4f44_a792_64304f6e7426
        );
    }

    #[test]
    fn text_service_description_is_stable() {
        assert_eq!(TEXT_SERVICE_DESCRIPTION, "Flower Pinyin input method");
    }
}
