#![cfg_attr(not(target_os = "windows"), forbid(unsafe_code))]
//! Windows Text Services Framework boundary for Flower.
//!
//! The TSF/COM module is compiled only for Windows. It exports the in-process
//! server entry points, a class factory for the Flower text service, and a
//! text service that maintains per-context engine sessions and applies
//! composition updates through synchronous TSF edit sessions.

pub const TEXT_SERVICE_DESCRIPTION: &str = crate::registration::FLOWER_TEXT_SERVICE_DESCRIPTION;

pub fn is_supported_platform() -> bool {
    cfg!(target_os = "windows")
}

#[cfg(target_os = "windows")]
pub const FLOWER_TEXT_SERVICE_CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(crate::registration::FLOWER_TEXT_SERVICE_CLSID_U128);

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod windows_exports {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread::ThreadId;

    use crate::engine::{
        map_windows_virtual_key, CompositionUpdate, EngineRouter, KeyEvent, Modifiers,
    };
    use crate::engine::{promote_order, Lexicon, RankingOrder};
    use crate::platform::candidate::{
        deferred_action_is_valid, edit_publish_decision, AcceptedCandidateAction,
        CandidatePopupAction, DeferredActionState, DeferredActionTicket, EditPublishDecision,
        PopupRect,
    };
    use crate::platform::composition::{CompositionController, CompositionHost};
    use crate::platform::diagnostics::{record, record_result, Stage};
    use crate::platform::{CandidatePopupModel, CandidatePopupWindow, PopupError};
    use crate::settings::{Settings, SettingsStore};
    use crate::user_dictionary::{DictionaryStore, LearningWorker};
    use windows::core::{implement, AsImpl, Error, IUnknown, Interface, Result, GUID, HRESULT};
    use windows::Win32::Foundation::{
        BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER,
        E_UNEXPECTED, FALSE, LPARAM, POINT, RECT, S_FALSE, S_OK, TRUE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::TextServices::{
        ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
        ITfContextComposition, ITfContextView, ITfEditSession, ITfEditSession_Impl,
        ITfInsertAtSelection, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfRange,
        ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfThreadMgr, TF_AE_NONE, TF_ANCHOR_END,
        TF_ES_ASYNC, TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY, TF_SELECTION,
        TF_SELECTIONSTYLE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetGUIThreadInfo, GetWindowThreadProcessId, IsChild, GUITHREADINFO,
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

    type ActivationToken = u64;

    static NEXT_ACTIVATION_TOKEN: AtomicU64 = AtomicU64::new(1);

    fn next_activation_token() -> ActivationToken {
        NEXT_ACTIVATION_TOKEN.fetch_add(1, Ordering::SeqCst)
    }

    /// A popup window owned by the thread that created it.
    ///
    /// The registration guard keeps `DllCanUnloadNow` false for as long as the
    /// window state exists, including while it is leased out of thread-local
    /// storage for a Win32 call.
    struct ThreadPopup {
        window: CandidatePopupWindow,
        _registration: ObjectRegistration,
    }

    thread_local! {
        static THREAD_POPUPS: RefCell<HashMap<ActivationToken, ThreadPopup>> =
            RefCell::new(HashMap::new());
    }

    fn register_thread_popup(token: ActivationToken, window: CandidatePopupWindow) -> bool {
        let popup = ThreadPopup {
            window,
            _registration: ObjectRegistration::new(),
        };
        match THREAD_POPUPS.try_with(|popups| popups.borrow_mut().insert(token, popup)) {
            Ok(displaced) => {
                // Drop outside the RefCell borrow so a displaced window never
                // runs Win32 destruction while thread-local storage is
                // borrowed.
                drop(displaced);
                true
            }
            Err(_) => false,
        }
    }

    /// Leases a window from the owning thread. The RefCell borrow ends before
    /// the caller performs any Win32 calls, and a non-owner thread is never
    /// given access to an `HWND` created elsewhere.
    fn take_thread_popup(token: ActivationToken, owner_thread: ThreadId) -> Option<ThreadPopup> {
        if std::thread::current().id() != owner_thread {
            return None;
        }

        THREAD_POPUPS
            .try_with(|popups| popups.borrow_mut().remove(&token))
            .ok()
            .flatten()
    }

    fn return_thread_popup(token: ActivationToken, owner_thread: ThreadId, popup: ThreadPopup) {
        if std::thread::current().id() != owner_thread {
            drop(popup);
            return;
        }

        let displaced = THREAD_POPUPS.try_with(|popups| popups.borrow_mut().insert(token, popup));
        drop(displaced);
    }

    fn close_thread_popup(token: ActivationToken, owner_thread: ThreadId) {
        let Some(mut popup) = take_thread_popup(token, owner_thread) else {
            return;
        };

        popup.window.hide();
        let _ = popup.window.destroy();
        drop(popup);
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
    struct ActiveComposition {
        handle: ITfComposition,
        terminated: Rc<Cell<bool>>,
    }

    impl std::ops::Deref for ActiveComposition {
        type Target = ITfComposition;

        fn deref(&self) -> &Self::Target {
            &self.handle
        }
    }

    #[implement(ITfCompositionSink)]
    struct CompositionSink {
        terminated: Rc<Cell<bool>>,
        owner_thread: ThreadId,
        _registration: ObjectRegistration,
    }

    impl ITfCompositionSink_Impl for CompositionSink_Impl {
        fn OnCompositionTerminated(
            &self,
            _edit_cookie: u32,
            _composition: Option<&ITfComposition>,
        ) -> Result<()> {
            if std::thread::current().id() != self.owner_thread {
                return Err(E_UNEXPECTED.into());
            }
            self.terminated.set(true);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct ContextEntry {
        context: ITfContext,
        router: EngineRouter,
        composition: Option<ActiveComposition>,
        controller: CompositionController,
        popup_model: CandidatePopupModel,
    }

    impl ContextEntry {
        fn composition_terminated(&self) -> bool {
            self.composition
                .as_ref()
                .is_some_and(|composition| composition.terminated.get())
        }

        fn new(context: &ITfContext, settings: &Settings, lexicon: &Lexicon) -> Option<Self> {
            let mut router =
                EngineRouter::from_lexicon(lexicon.clone(), settings.candidate_page_size).ok()?;
            router.set_candidate_width(Some(crate::engine::paging::DEFAULT_CANDIDATE_WIDTH));
            Some(Self {
                context: context.clone(),
                router,
                composition: None,
                controller: CompositionController::new(),
                popup_model: CandidatePopupModel::new(),
            })
        }
    }

    struct Activation {
        thread_manager: ITfThreadMgr,
        keystroke_manager: ITfKeystrokeMgr,
        client_id: u32,
        contexts: HashMap<ContextKey, ContextEntry>,
        /// Unique identity for this activation and its thread-local window.
        token: ActivationToken,
        /// Thread that created the service and owns the popup window.
        owner_thread: ThreadId,
        /// Incremented on every focus transition so stale key work cannot
        /// republish or show a popup after focus changed.
        focus_epoch: u64,
        /// 当前 TSF 服务是否拥有前台焦点。
        foreground: bool,
        shown_context: Option<ContextKey>,
        /// True while a key callback owns the synchronous edit session.
        /// Recursive key callbacks pass through instead of starting another.
        processing: bool,
        /// Immutable settings captured when this activation started.
        settings: Settings,
        lexicon: Lexicon,
        rankings: Arc<Vec<RankingOrder>>,
        adaptive_ranking_enabled: bool,
        learning: Option<LearningWorker>,
        _learning_registration: Option<ObjectRegistration>,
        local_data_error: bool,
    }

    type SharedActivation = Rc<RefCell<Option<Activation>>>;

    fn claim_promotion(claim: &Cell<bool>, allowed: bool) -> bool {
        allowed && !claim.replace(true)
    }

    #[cfg(test)]
    #[test]
    fn unsafe_or_duplicate_completion_never_claims_a_second_promotion() {
        let claim = Cell::new(false);
        assert!(!claim_promotion(&claim, false));
        assert!(!claim.get());
        assert!(claim_promotion(&claim, true));
        assert!(!claim_promotion(&claim, true));
    }

    fn sync_idle_rankings(
        shared: &SharedActivation,
        token: ActivationToken,
        router: &mut EngineRouter,
    ) {
        if router.is_composing() {
            return;
        }
        if let Ok(activation) = shared.try_borrow() {
            if let Some(active) = activation.as_ref().filter(|a| a.token == token) {
                router.set_ranking_orders(Arc::clone(&active.rankings));
            }
        }
    }

    fn context_key(context: &ITfContext) -> ContextKey {
        // TSF supplies the same COM interface identity across callbacks for a
        // context. The raw interface pointer is stable; a Rust reference
        // address is not.
        context.as_raw() as usize
    }

    /// 查询当前 TSF 焦点文档的 top context。
    ///
    /// 调用方不得同时借用候选激活状态。`None` 表示查询失败或当前没有焦点
    /// context，二者都必须使延迟鼠标动作失效。
    fn focused_context_key(thread_manager: &ITfThreadMgr) -> Option<ContextKey> {
        // SAFETY: The thread manager is retained for the activation lifetime.
        // `GetFocus` and `GetTop` return counted COM references.
        unsafe {
            let document = thread_manager.GetFocus().ok()?;
            let context = document.GetTop().ok()?;
            Some(context_key(&context))
        }
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

    /// Maps a popup construction failure to the HRESULT reported by `Activate`.
    fn popup_activation_hresult(error: &PopupError) -> HRESULT {
        match error {
            PopupError::Win32 { source, .. } => source.code(),
            PopupError::NoSnapshot | PopupError::Destroyed => E_UNEXPECTED,
        }
    }

    struct TfCompositionHost {
        context: ITfContext,
        composition: Option<ActiveComposition>,
        edit_cookie: u32,
    }

    impl TfCompositionHost {
        fn new(
            context: ITfContext,
            composition: Option<ActiveComposition>,
            edit_cookie: u32,
        ) -> Self {
            Self {
                context,
                composition,
                edit_cookie,
            }
        }

        fn into_composition(self) -> Option<ActiveComposition> {
            self.composition
        }

        fn text_range(&mut self) -> Result<ITfRange> {
            let Some(composition) = self.composition.clone() else {
                return Err(E_UNEXPECTED.into());
            };
            if composition.terminated.get() {
                return Err(E_UNEXPECTED.into());
            }

            // SAFETY: The composition is a live TSF object for this context.
            match unsafe { composition.GetRange() } {
                Ok(range) => Ok(range),
                Err(error) => {
                    self.end_composition_best_effort();
                    Err(error)
                }
            }
        }

        fn move_selection_to_end(&self, range: &ITfRange) -> Result<()> {
            let caret = unsafe { range.Clone() }?;
            unsafe { caret.Collapse(self.edit_cookie, TF_ANCHOR_END) }?;
            let selection = TF_SELECTION {
                range: std::mem::ManuallyDrop::new(Some(caret)),
                style: TF_SELECTIONSTYLE {
                    ase: TF_AE_NONE,
                    fInterimChar: FALSE,
                },
            };
            let result = unsafe {
                self.context
                    .SetSelection(self.edit_cookie, std::slice::from_ref(&selection))
            };
            drop(std::mem::ManuallyDrop::into_inner(selection.range));
            result
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
            let insert: Result<ITfInsertAtSelection> = self.context.cast();
            record_result(Stage::QueryInsert, &insert);
            let insert = insert?;
            // SAFETY: A query-only insertion returns a live range at the
            // selection without changing document text.
            let range =
                unsafe { insert.InsertTextAtSelection(self.edit_cookie, TF_IAS_QUERYONLY, &[]) };
            record_result(Stage::QueryInsertionRange, &range);
            let range = range?;
            let composition_manager: ITfContextComposition = self.context.cast()?;
            let terminated = Rc::new(Cell::new(false));
            let sink: ITfCompositionSink = CompositionSink {
                terminated: Rc::clone(&terminated),
                owner_thread: std::thread::current().id(),
                _registration: ObjectRegistration::new(),
            }
            .into();

            // SAFETY: The queried range and context are live in the current
            // read/write edit session.
            let composition =
                unsafe { composition_manager.StartComposition(self.edit_cookie, &range, &sink) };
            record_result(Stage::StartComposition, &composition);
            let composition = ActiveComposition {
                handle: composition?,
                terminated,
            };
            self.composition = Some(composition.clone());

            let text: Vec<u16> = preedit.encode_utf16().collect();
            // SAFETY: The new composition belongs to this edit session and the
            // UTF-16 buffer remains alive for the duration of the call.
            let set_text = unsafe { composition.GetRange() }.and_then(|range| {
                // SAFETY: `range` is live and `text` is a valid UTF-16 buffer.
                unsafe { range.SetText(self.edit_cookie, 0, &text) }?;
                self.move_selection_to_end(&range)
            });
            record_result(Stage::SetInitialText, &set_text);

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
            unsafe { range.SetText(self.edit_cookie, 0, &text) }?;
            self.move_selection_to_end(&range)
        }

        fn commit(&mut self, text: &str) -> Result<()> {
            let range = self.text_range()?;
            let text: Vec<u16> = text.encode_utf16().collect();
            // SAFETY: The range belongs to the active composition, and the
            // UTF-16 buffer remains alive for the duration of the call.
            unsafe { range.SetText(self.edit_cookie, 0, &text) }?;
            self.move_selection_to_end(&range)?;
            self.end_composition()
        }

        fn cancel(&mut self) -> Result<()> {
            let range = self.text_range()?;
            // SAFETY: The range belongs to the active composition and this edit
            // session owns its read/write cookie. Clearing it removes only the
            // preedit inserted by Flower before ending the composition.
            unsafe { range.SetText(self.edit_cookie, 0, &[]) }?;
            self.move_selection_to_end(&range)?;
            self.end_composition()
        }
    }

    #[derive(Clone)]
    struct EditSessionOutcome {
        composition: Option<ActiveComposition>,
        controller: CompositionController,
        caret: Option<RECT>,
        learning_allowed: bool,
    }

    fn learning_scope_value_is_safe(scope: i32) -> bool {
        matches!(scope, 0 | 57 | 58)
    }

    #[cfg(test)]
    #[test]
    fn learning_scope_policy_rejects_password_private_and_unknown_values() {
        for scope in [0, 57, 58] {
            assert!(learning_scope_value_is_safe(scope));
        }
        for scope in [-1, 31, 61, 999] {
            assert!(!learning_scope_value_is_safe(scope));
        }
    }

    fn scope_allows_learning(context: &ITfContext, cookie: u32) -> bool {
        use windows::Win32::UI::TextServices::{ITfInputScope, GUID_PROP_INPUTSCOPE};
        // Read the application's declaration over the actual selection inside
        // this edit session. Missing, unsupported, or mixed unsafe scopes deny
        // learning; never fall back to a different property channel.
        unsafe {
            let mut selection = [TF_SELECTION::default()];
            let mut fetched = 0;
            let result = context.GetSelection(cookie, u32::MAX, &mut selection, &mut fetched);
            let range = std::mem::ManuallyDrop::take(&mut selection[0].range);
            if result.is_err() || fetched != 1 {
                return false;
            }
            let Some(range) = range else {
                return false;
            };
            let property = context.GetAppProperty(&GUID_PROP_INPUTSCOPE);
            record_result(Stage::LearningAppProperty, &property);
            let Ok(property) = property else {
                return false;
            };
            let value = property.GetValue(cookie, &range);
            record_result(Stage::LearningScopeValue, &value);
            let Ok(value) = value else {
                return false;
            };
            let Ok(unknown) = windows::core::IUnknown::try_from(&value) else {
                return false;
            };
            let Ok(scope) = unknown.cast::<ITfInputScope>() else {
                return false;
            };
            let mut scopes = std::ptr::null_mut();
            let mut count = 0;
            if scope.GetInputScopes(&mut scopes, &mut count).is_err() {
                return false;
            }
            if scopes.is_null() {
                return false;
            }
            let allowed = count > 0
                && count <= 256
                && std::slice::from_raw_parts(scopes, count as usize)
                    .iter()
                    .all(|scope| learning_scope_value_is_safe(scope.0));
            windows::Win32::System::Com::CoTaskMemFree(Some(scopes.cast()));
            allowed
        }
    }

    fn show_local_data_error() {
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
        unsafe {
            let _ = MessageBoxW(None,
            windows::core::w!("Flower 的本地词典加载或词频保存失败；已提交的文字不受影响。请打开 Flower 设置检查用户词典，原文件不会被重建或覆盖。"),
            windows::core::w!("Flower 本地词典"), MB_OK | MB_ICONWARNING);
        }
    }

    #[derive(Clone)]
    struct EditTarget {
        promotion_claim: Rc<Cell<bool>>,
        activation: SharedActivation,
        thread_manager: ITfThreadMgr,
        token: ActivationToken,
        owner_thread: ThreadId,
        key: ContextKey,
        focus_epoch: u64,
    }

    /// 异步鼠标编辑完成后发布状态所需的数据。
    ///
    /// 该结构不持有 `TextService` 的 COM 强引用，只持有一个可失效的激活状态
    /// 线程本地 `Rc<RefCell<_>>` 和 RAII processing guard，因此不会形成服务与
    /// 窗口回调环，也不会被错误线程共享。
    struct PopupActionCompletion {
        target: EditTarget,
        expected_revision: u64,
        entry: Option<ContextEntry>,
        update: Option<CompositionUpdate>,
        completed: bool,
        _processing: ProcessingGuard,
    }

    struct EditSessionState {
        composition: Option<ActiveComposition>,
        controller: CompositionController,
        outcome: Option<EditSessionOutcome>,
    }

    #[implement(ITfEditSession)]
    struct EditSession {
        context: ITfContext,
        update: CompositionUpdate,
        state: Mutex<EditSessionState>,
        completion: Mutex<Option<PopupActionCompletion>>,
        requires_preflight: bool,
        _registration: ObjectRegistration,
    }

    impl EditSession {
        fn new(
            context: ITfContext,
            update: CompositionUpdate,
            composition: Option<ActiveComposition>,
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
                completion: Mutex::new(None),
                requires_preflight: false,
                _registration: ObjectRegistration::new(),
            }
        }

        fn new_popup_action(
            context: ITfContext,
            update: CompositionUpdate,
            composition: Option<ActiveComposition>,
            controller: CompositionController,
            completion: PopupActionCompletion,
        ) -> Self {
            Self {
                context,
                update,
                state: Mutex::new(EditSessionState {
                    composition,
                    controller,
                    outcome: None,
                }),
                completion: Mutex::new(Some(completion)),
                requires_preflight: true,
                _registration: ObjectRegistration::new(),
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

        fn take_completion(&self) -> Option<PopupActionCompletion> {
            match self.completion.lock() {
                Ok(mut guard) => guard.take(),
                Err(poisoned) => poisoned.into_inner().take(),
            }
        }
    }

    impl ITfEditSession_Impl for EditSession_Impl {
        fn DoEditSession(&self, edit_cookie: u32) -> Result<()> {
            record(Stage::DoEditSession, S_OK);
            let this = self.get_impl();
            let completion = this.take_completion();
            if this.requires_preflight && completion.is_none() {
                return Err(E_UNEXPECTED.into());
            }
            if completion
                .as_ref()
                .is_some_and(|completion| !completion.preflight())
            {
                if let Some(completion) = completion {
                    completion.invalidate();
                }
                return Err(E_UNEXPECTED.into());
            }

            let (composition, controller) = {
                let state = this.lock_state();
                (state.composition.clone(), state.controller)
            };
            if composition
                .as_ref()
                .is_some_and(|composition| composition.terminated.get())
            {
                return Err(E_UNEXPECTED.into());
            }
            let learning_allowed =
                this.update.commit.is_some() && scope_allows_learning(&this.context, edit_cookie);
            let mut host = TfCompositionHost::new(this.context.clone(), composition, edit_cookie);
            let mut controller = controller;
            let result = controller
                .apply_update(&mut host, &this.update)
                .and_then(|()| {
                    if controller.is_active()
                        && host
                            .composition
                            .as_ref()
                            .is_some_and(|composition| composition.terminated.get())
                    {
                        Err(E_UNEXPECTED.into())
                    } else {
                        Ok(())
                    }
                });
            record_result(Stage::ApplyUpdate, &result);
            let caret = if result.is_ok() && controller.is_active() {
                host.composition.as_ref().and_then(|composition| {
                    composition_caret(&this.context, composition, edit_cookie)
                })
            } else {
                None
            };
            let composition = host.into_composition();

            let mut state = this.lock_state();
            state.composition = composition.clone();
            let outcome = if result.is_ok() {
                Some(EditSessionOutcome {
                    composition,
                    controller,
                    caret,
                    learning_allowed,
                })
            } else {
                None
            };
            if result.is_ok() {
                state.controller = controller;
                state.outcome = outcome.clone();
            }
            drop(state);
            if let Some(completion) = completion {
                completion.finish(result.is_ok(), outcome);
            }
            result
        }
    }

    /// Returns the best-effort screen-space caret rectangle at the end of the
    /// composition preedit.
    ///
    /// This runs inside the synchronous TSF edit session, so the context view
    /// and composition range are valid. Failure only suppresses popup
    /// positioning; the document edit has already succeeded.
    fn composition_caret(
        context: &ITfContext,
        composition: &ITfComposition,
        edit_cookie: u32,
    ) -> Option<RECT> {
        // All TSF range and view queries stay inside the current edit session.
        // A layout miss must not suppress otherwise valid candidates forever.
        unsafe {
            let view: ITfContextView = context.GetActiveView().ok()?;
            if let Some(collapsed) = composition
                .GetRange()
                .ok()
                .and_then(|range| collapsed_range_end(&range, edit_cookie))
            {
                let mut rect = RECT::default();
                let mut clipped = BOOL::default();
                let result = view.GetTextExt(edit_cookie, &collapsed, &mut rect, &mut clipped);
                record_result(Stage::LayoutTextExt, &result);
                if result.is_ok() {
                    if clipped.as_bool() {
                        return None;
                    }
                    if valid_caret_rect(rect) {
                        return Some(rect);
                    }
                }
            }
            let viewport = view
                .GetScreenExt()
                .ok()
                .filter(|r| r.right > r.left && r.bottom > r.top)?;
            if let Some(rect) = gui_caret_for_view(&view, viewport) {
                record(Stage::AnchorGui, S_OK);
                return Some(rect);
            }
            record(Stage::AnchorView, S_OK);
            viewport_edge_anchor(viewport)
        }
    }

    fn valid_caret_rect(rect: RECT) -> bool {
        rect.right >= rect.left && rect.bottom > rect.top
    }

    fn viewport_edge_anchor(rect: RECT) -> Option<RECT> {
        (rect.right > rect.left && rect.bottom > rect.top).then(|| RECT {
            left: rect.left,
            right: rect.left,
            top: rect.bottom.saturating_sub(1),
            bottom: rect.bottom,
        })
    }

    #[cfg(test)]
    #[test]
    fn caret_geometry_accepts_zero_width_but_never_empty_or_inverted_viewports() {
        assert!(valid_caret_rect(RECT {
            left: -20,
            top: 10,
            right: -20,
            bottom: 26
        }));
        for rect in [
            RECT::default(),
            RECT {
                left: 20,
                top: 10,
                right: 19,
                bottom: 26,
            },
            RECT {
                left: 20,
                top: 10,
                right: 22,
                bottom: 10,
            },
        ] {
            assert!(!valid_caret_rect(rect));
        }
        assert!(viewport_edge_anchor(RECT::default()).is_none());
        assert!(viewport_edge_anchor(RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 200
        })
        .is_none());
        let anchor = viewport_edge_anchor(RECT {
            left: -1920,
            top: -200,
            right: -1000,
            bottom: 600,
        })
        .unwrap();
        assert_eq!(
            (anchor.left, anchor.right, anchor.top, anchor.bottom),
            (-1920, -1920, 599, 600)
        );
    }

    unsafe fn gui_caret_for_view(view: &ITfContextView, viewport: RECT) -> Option<RECT> {
        let window = view.GetWnd().ok()?;
        if window.0.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(window, None);
        if thread == 0 {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut info).ok()?;
        let caret_window = info.hwndCaret;
        if caret_window.0.is_null()
            || !(caret_window == window || IsChild(window, caret_window).as_bool())
            || !(info.hwndFocus == window || IsChild(window, info.hwndFocus).as_bool())
        {
            return None;
        }
        let mut top_left = POINT {
            x: info.rcCaret.left,
            y: info.rcCaret.top,
        };
        let mut bottom_right = POINT {
            x: info.rcCaret.right,
            y: info.rcCaret.bottom,
        };
        if !ClientToScreen(caret_window, &mut top_left).as_bool()
            || !ClientToScreen(caret_window, &mut bottom_right).as_bool()
        {
            return None;
        }
        let rect = RECT {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        };
        (valid_caret_rect(rect)
            && rect.left >= viewport.left
            && rect.right <= viewport.right
            && rect.top >= viewport.top
            && rect.bottom <= viewport.bottom)
            .then_some(rect)
    }

    /// Clones a range and collapses it to its end for caret positioning.
    fn collapsed_range_end(range: &ITfRange, edit_cookie: u32) -> Option<ITfRange> {
        // SAFETY: `range` is a live range owned by the current edit session.
        // `ITfRange::Clone` creates an independent TSF range object, so
        // collapsing the clone cannot modify the composition range. Failure to
        // clone or collapse only suppresses popup positioning.
        unsafe {
            let collapsed = range.Clone().ok()?;
            collapsed.Collapse(edit_cookie, TF_ANCHOR_END).ok()?;
            Some(collapsed)
        }
    }

    /// 请求一次同步键盘编辑并取回编辑会话结果。
    ///
    /// TSF `SetText` 与 `EndComposition` 不是单一事务：即使调用返回错误，
    /// 宿主文档也可能已经被部分修改。调用方只能恢复引擎侧快照，不能把
    /// `None` 解释为“文档保证未变”或执行自动回滚。
    fn request_key_edit_session(
        context: &ITfContext,
        client_id: u32,
        update: CompositionUpdate,
        composition: Option<ActiveComposition>,
        controller: CompositionController,
    ) -> Option<EditSessionOutcome> {
        let session: ITfEditSession =
            EditSession::new(context.clone(), update, composition, controller).into();

        // SAFETY: The context is live and `session` remains alive until the
        // synchronous request returns.
        let requested = unsafe {
            context.RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READWRITE)
        };
        record_result(Stage::RequestKeyEdit, &requested);
        if let Ok(status) = requested.as_ref() {
            record(Stage::EditSessionResult, *status);
        }
        match requested {
            // SAFETY: `session` was created from `EditSession` above.
            Ok(result) if result.is_ok() => unsafe {
                <ITfEditSession as AsImpl<EditSession>>::as_impl(&session).take_outcome()
            },
            Ok(_) | Err(_) => None,
        }
    }

    fn dispatch_popup_action(
        shared: &SharedActivation,
        token: ActivationToken,
        owner_thread: ThreadId,
        action: CandidatePopupAction,
    ) {
        if std::thread::current().id() != owner_thread {
            return;
        }

        let initial = {
            let Ok(activation) = shared.try_borrow() else {
                return;
            };
            let Some(active) = activation
                .as_ref()
                .filter(|active| active.token == token && active.foreground)
            else {
                return;
            };
            if active.processing {
                return;
            }
            let Some(key) = active.shown_context else {
                return;
            };
            let Some(entry) = active.contexts.get(&key) else {
                return;
            };
            let Some(snapshot) = entry.popup_model.snapshot() else {
                return;
            };
            if entry.popup_model.accept_action(&action).is_err() {
                return;
            }
            (
                active.thread_manager.clone(),
                key,
                active.focus_epoch,
                snapshot.revision,
            )
        };
        let (thread_manager, key, focus_epoch, expected_revision) = initial;

        // Query TSF focus without borrowing the thread-local activation.
        let focus_context_matches = focused_context_key(&thread_manager) == Some(key);
        if !focus_context_matches {
            clear_invalid_popup_action(shared, token, owner_thread, key);
            return;
        }

        let ticket = DeferredActionTicket {
            activation: token,
            focus_epoch,
            context: key,
            revision: expected_revision,
        };
        let prepared = {
            let Ok(mut activation) = shared.try_borrow_mut() else {
                return;
            };
            let Some(active) = activation.as_mut().filter(|active| active.token == token) else {
                return;
            };
            if active.processing {
                return;
            }
            let state = deferred_action_state(active, ticket, owner_thread, focus_context_matches);
            if !deferred_action_is_valid(ticket, state) {
                None
            } else {
                let Some(entry) = active.contexts.get(&key) else {
                    return;
                };
                let Ok(accepted) = entry.popup_model.accept_action(&action) else {
                    return;
                };
                let entry = entry.clone();
                active.processing = true;
                let processing = ProcessingGuard {
                    activation: Rc::clone(shared),
                    token,
                    owner_thread,
                };
                Some((
                    entry,
                    active.client_id,
                    thread_manager,
                    accepted,
                    processing,
                ))
            }
        };
        let Some((old_entry, client_id, thread_manager, accepted, processing)) = prepared else {
            clear_invalid_popup_action(shared, token, owner_thread, key);
            return;
        };

        let mut entry = old_entry;
        let update = match accepted {
            AcceptedCandidateAction::Select { index, .. } => entry.router.select_candidate(index),
            AcceptedCandidateAction::PreviousPage { .. } => entry.router.previous_page(),
            AcceptedCandidateAction::NextPage { .. } => entry.router.next_page(),
        };
        let update = match update {
            Ok(update) => update,
            Err(_) => {
                clear_invalid_popup_action(shared, token, owner_thread, key);
                return;
            }
        };

        let update_for_completion = update.clone();
        let context = entry.context.clone();
        let composition = entry.composition.take();
        let controller = entry.controller;
        let target = EditTarget {
            promotion_claim: Rc::new(Cell::new(false)),
            activation: Rc::clone(shared),
            thread_manager,
            token,
            owner_thread,
            key,
            focus_epoch,
        };
        let completion = PopupActionCompletion {
            target,
            expected_revision,
            entry: Some(entry),
            update: Some(update_for_completion),
            completed: false,
            _processing: processing,
        };
        let session: ITfEditSession = EditSession::new_popup_action(
            context.clone(),
            update,
            composition,
            controller,
            completion,
        )
        .into();

        // SAFETY: The context belongs to the retained activation entry and
        // remains live through the COM session reference. Async dispatch
        // prevents document edits from running inside the popup WndProc.
        let requested = unsafe {
            context.RequestEditSession(client_id, &session, TF_ES_ASYNC | TF_ES_READWRITE)
        };
        if !matches!(&requested, Ok(result) if result.is_ok()) {
            // SAFETY: `session` was created from `EditSession` above.
            if let Some(completion) = unsafe {
                <ITfEditSession as AsImpl<EditSession>>::as_impl(&session).take_completion()
            } {
                completion.invalidate();
            }
        }
    }

    fn deferred_action_state(
        active: &Activation,
        ticket: DeferredActionTicket,
        owner_thread: ThreadId,
        focus_context_matches: bool,
    ) -> DeferredActionState {
        DeferredActionState {
            owner_thread_matches: std::thread::current().id() == owner_thread,
            activation: Some(active.token),
            foreground: active.foreground,
            focus_epoch: active.focus_epoch,
            focus_context_matches,
            shown_context: active.shown_context,
            revision: active
                .contexts
                .get(&ticket.context)
                .and_then(|entry| entry.popup_model.snapshot())
                .map(|snapshot| snapshot.revision),
        }
    }

    fn clear_invalid_popup_action(
        shared: &SharedActivation,
        token: ActivationToken,
        owner_thread: ThreadId,
        key: ContextKey,
    ) {
        if std::thread::current().id() != owner_thread {
            return;
        }
        {
            let Ok(mut activation) = shared.try_borrow_mut() else {
                return;
            };
            if let Some(active) = activation
                .as_mut()
                .filter(|active| active.token == token && active.shown_context == Some(key))
            {
                active.shown_context = None;
            }
        }

        let Some(mut popup) = take_thread_popup(token, owner_thread) else {
            return;
        };
        popup.window.cancel_pending_actions();
        popup.window.hide();
        let still_owned = shared.try_borrow().ok().is_some_and(
            |activation| matches!(activation.as_ref(), Some(active) if active.token == token),
        );
        if still_owned {
            return_thread_popup(token, owner_thread, popup);
        }
    }

    #[implement(ITfTextInputProcessor, ITfKeyEventSink)]
    struct TextService {
        activation: SharedActivation,
        owner_thread: ThreadId,
        _registration: ObjectRegistration,
    }

    impl TextService {
        fn new() -> Self {
            Self {
                activation: Rc::new(RefCell::new(None)),
                owner_thread: std::thread::current().id(),
                _registration: ObjectRegistration::new(),
            }
        }

        fn owns_current_thread(&self) -> bool {
            std::thread::current().id() == self.owner_thread
        }

        /// Restores a context entry only when the activation that started the
        /// key is still active. The entry and any displaced entry are dropped
        /// after the activation borrow is released.
        fn restore_context(&self, token: ActivationToken, key: ContextKey, entry: ContextEntry) {
            restore_activation_context(&self.activation, token, key, entry);
        }
    }

    fn restore_activation_context(
        shared: &SharedActivation,
        token: ActivationToken,
        key: ContextKey,
        entry: ContextEntry,
    ) {
        let mut entry = Some(entry);
        let displaced = {
            let Ok(mut activation) = shared.try_borrow_mut() else {
                drop(entry);
                return;
            };
            activation
                .as_mut()
                .filter(|active| active.token == token)
                .and_then(|active| {
                    entry
                        .take()
                        .and_then(|entry| active.contexts.insert(key, entry))
                })
        };

        // TSF objects and popup state are released after the activation borrow.
        drop(entry);
        drop(displaced);
    }

    /// Clears an activation's in-flight edit flag when the callback exits.
    ///
    /// The flag is cleared only while the same activation token is still
    /// present, so a callback that outlived a Deactivate/Activate cycle cannot
    /// modify the replacement activation.
    struct ProcessingGuard {
        activation: SharedActivation,
        token: ActivationToken,
        owner_thread: ThreadId,
    }

    impl Drop for ProcessingGuard {
        fn drop(&mut self) {
            if std::thread::current().id() != self.owner_thread {
                return;
            }
            let token = self.token;
            let Ok(mut activation) = self.activation.try_borrow_mut() else {
                return;
            };
            if let Some(active) = activation.as_mut().filter(|active| active.token == token) {
                active.processing = false;
            }
        }
    }

    /// 将一次成功编辑的 router/popup 状态发布回当前激活。
    ///
    /// 键盘和鼠标共用此路径。Win32 调用发生在激活借用之外；重入按键由
    /// `ProcessingGuard` 的 processing 标志拒绝。
    fn publish_successful_edit(
        target: &EditTarget,
        mut entry: ContextEntry,
        update: &CompositionUpdate,
        outcome: EditSessionOutcome,
    ) {
        if std::thread::current().id() != target.owner_thread {
            drop(entry);
            return;
        }
        if outcome.controller.is_active()
            && outcome
                .composition
                .as_ref()
                .is_some_and(|composition| composition.terminated.get())
        {
            clear_invalid_popup_action(
                &target.activation,
                target.token,
                target.owner_thread,
                target.key,
            );
            return;
        }
        let learning_allowed = outcome.learning_allowed;
        let events = if update.commit.is_some() {
            entry.router.pending_promotions().to_vec()
        } else {
            Vec::new()
        };
        entry.composition = outcome.composition;
        entry.controller = outcome.controller;
        let caret = outcome.caret;

        let focus_context_matches = focused_context_key(&target.thread_manager) == Some(target.key);
        let (decision, shown_context) = {
            let Ok(activation) = target.activation.try_borrow() else {
                drop(entry);
                return;
            };
            let (current_token, current_epoch, shown_context) = match activation.as_ref() {
                Some(active) => (Some(active.token), active.focus_epoch, active.shown_context),
                None => (None, 0, None),
            };
            (
                edit_publish_decision(
                    current_token,
                    target.token,
                    current_epoch,
                    target.focus_epoch,
                ),
                shown_context,
            )
        };

        if decision == EditPublishDecision::Discard {
            // 新的激活不能接收旧 token 的编辑结果。
            drop(entry);
            return;
        }

        let allow_show = decision == EditPublishDecision::PublishAndShow && focus_context_matches;
        let mut popup = take_thread_popup(target.token, target.owner_thread);
        let mut visible = false;
        let mut display_update = update.clone();
        if entry.router.is_composing() {
            if let (Some(popup), Some(caret)) = (popup.as_ref(), caret) {
                if let Ok(width) = popup.window.candidate_width_budget(PopupRect::new(
                    caret.left,
                    caret.top,
                    caret.right,
                    caret.bottom,
                )) {
                    display_update = entry.router.set_candidate_width(Some(width));
                }
            }
        }
        entry.popup_model.apply_update(&display_update);

        if let Some(popup) = popup.as_mut() {
            if shown_context != Some(target.key) {
                popup.window.hide();
            }

            if entry
                .popup_model
                .snapshot()
                .is_some_and(|snapshot| popup.window.update(snapshot).is_ok())
            {
                let can_show_now = allow_show && {
                    target
                        .activation
                        .try_borrow()
                        .ok()
                        .is_some_and(|activation| {
                            matches!(
                                activation.as_ref(),
                                Some(active)
                                    if active.token == target.token
                                        && active.focus_epoch == target.focus_epoch
                            )
                        })
                };
                visible = can_show_now
                    && caret.is_some_and(|caret| {
                        popup
                            .window
                            .show_at(PopupRect::new(
                                caret.left,
                                caret.top,
                                caret.right,
                                caret.bottom,
                            ))
                            .is_ok()
                    });
            }

            if !visible {
                popup.window.hide();
            }
        }

        // Publish the entry and popup visibility with one short borrow. Focus or
        // activation changes observed here win over the captured key state.
        let mut entry = Some(entry);
        let mut displaced = None;
        let mut keep_popup = false;
        let mut hide_now = visible;
        {
            let Ok(mut activation) = target.activation.try_borrow_mut() else {
                drop(entry);
                return;
            };
            if let Some(active) = activation
                .as_mut()
                .filter(|active| active.token == target.token)
            {
                keep_popup = true;
                let still_focused = active.foreground
                    && active.focus_epoch == target.focus_epoch
                    && focus_context_matches;
                if !events.is_empty()
                    && claim_promotion(
                        &target.promotion_claim,
                        still_focused && learning_allowed && active.adaptive_ranking_enabled,
                    )
                {
                    let mut orders = active.rankings.as_ref().clone();
                    for event in &events {
                        promote_order(&mut orders, event);
                    }
                    if orders.iter().map(|o| o.candidates.len()).sum::<usize>() <= 100_000 {
                        active.rankings = Arc::new(orders);
                    } else {
                        active.local_data_error = true;
                    }
                    if let Some(worker) = active.learning.as_ref() {
                        if worker.submit_promotions(&events).is_err() {
                            active.local_data_error = true;
                        }
                    }
                }
                if visible && still_focused {
                    active.shown_context = Some(target.key);
                    hide_now = false;
                } else {
                    active.shown_context = None;
                }
                if let Some(mut entry) = entry.take() {
                    if !entry.router.is_composing() {
                        entry
                            .router
                            .set_ranking_orders(Arc::clone(&active.rankings));
                    }
                    displaced = active.contexts.insert(target.key, entry);
                }
            }
        }
        drop(entry);
        drop(displaced);

        if let Some(popup) = popup {
            if hide_now {
                popup.window.hide();
                keep_popup = target
                    .activation
                    .try_borrow()
                    .ok()
                    .is_some_and(|activation| {
                        matches!(
                            activation.as_ref(),
                            Some(active) if active.token == target.token
                        )
                    });
            }

            if keep_popup {
                return_thread_popup(target.token, target.owner_thread, popup);
            } else {
                // Activation ended while the popup was leased; destroy outside
                // the activation borrow.
                drop(popup);
            }
        }
    }

    impl PopupActionCompletion {
        fn preflight(&self) -> bool {
            if self
                .entry
                .as_ref()
                .is_some_and(ContextEntry::composition_terminated)
            {
                return false;
            }
            if std::thread::current().id() != self.target.owner_thread {
                return false;
            }
            let focus_context_matches =
                focused_context_key(&self.target.thread_manager) == Some(self.target.key);
            let ticket = DeferredActionTicket {
                activation: self.target.token,
                focus_epoch: self.target.focus_epoch,
                context: self.target.key,
                revision: self.expected_revision,
            };
            let Ok(activation) = self.target.activation.try_borrow() else {
                return false;
            };
            let Some(active) = activation
                .as_ref()
                .filter(|active| active.token == self.target.token)
            else {
                return false;
            };
            if !active.processing {
                return false;
            }
            deferred_action_is_valid(
                ticket,
                deferred_action_state(
                    active,
                    ticket,
                    self.target.owner_thread,
                    focus_context_matches,
                ),
            )
        }

        fn invalidate(mut self) {
            self.completed = true;
            clear_invalid_popup_action(
                &self.target.activation,
                self.target.token,
                self.target.owner_thread,
                self.target.key,
            );
        }

        fn finish(mut self, succeeded: bool, outcome: Option<EditSessionOutcome>) {
            self.completed = true;
            let Some(outcome) = outcome else {
                clear_invalid_popup_action(
                    &self.target.activation,
                    self.target.token,
                    self.target.owner_thread,
                    self.target.key,
                );
                return;
            };
            if !succeeded {
                clear_invalid_popup_action(
                    &self.target.activation,
                    self.target.token,
                    self.target.owner_thread,
                    self.target.key,
                );
                return;
            }
            let Some(entry) = self.entry.take() else {
                return;
            };
            let Some(update) = self.update.take() else {
                return;
            };
            publish_successful_edit(&self.target, entry, &update, outcome);
        }
    }

    impl Drop for PopupActionCompletion {
        fn drop(&mut self) {
            if !self.completed {
                clear_invalid_popup_action(
                    &self.target.activation,
                    self.target.token,
                    self.target.owner_thread,
                    self.target.key,
                );
            }
        }
    }

    impl Drop for TextService {
        fn drop(&mut self) {
            let active = self
                .activation
                .try_borrow_mut()
                .ok()
                .and_then(|mut activation| activation.take());

            if let Some(active) = active {
                // Best effort: only the owning thread can destroy the window.
                close_thread_popup(active.token, active.owner_thread);
                // Release TSF references after the activation borrow is gone.
                drop(active);
            }
        }
    }

    impl ITfTextInputProcessor_Impl for TextService_Impl {
        fn Activate(&self, thread_manager: Option<&ITfThreadMgr>, client_id: u32) -> Result<()> {
            record(Stage::Activate, S_OK);
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            let Some(thread_manager) = thread_manager else {
                return Err(E_INVALIDARG.into());
            };

            // Load configuration once per activation, before borrowing the
            // activation state and before creating its popup. Errors use
            // defaults without rewriting the configured file or surfacing a
            // host UI.
            let settings = SettingsStore::for_current_user()
                .and_then(|store| store.load())
                .unwrap_or_default();
            let mut lexicon = Lexicon::builtin().map_err(|_| Error::from(E_UNEXPECTED))?;
            let mut local_data_error = false;
            let mut learning = None;
            let mut rankings = Arc::new(Vec::new());
            let mut adaptive_ranking_enabled = false;
            match DictionaryStore::for_current_user()
                .and_then(|store| store.load().map(|data| (store, data)))
            {
                Ok((store, data)) => {
                    match data
                        .lexicon_tsv()
                        .ok()
                        .and_then(|tsv| lexicon.with_overlay(&tsv).ok())
                    {
                        Some(snapshot) => {
                            lexicon = snapshot;
                            adaptive_ranking_enabled = data.adaptive_ranking_enabled;
                            if adaptive_ranking_enabled {
                                rankings = Arc::new(data.rankings.clone());
                                match LearningWorker::start(store, data.generation) {
                                    Ok(worker) => learning = Some(worker),
                                    Err(_) => local_data_error = true,
                                }
                            }
                        }
                        None => local_data_error = true,
                    }
                }
                Err(_) => local_data_error = true,
            }
            let keystroke_manager: ITfKeystrokeMgr = thread_manager.cast()?;
            let key_sink: ITfKeyEventSink =
                <Self as ComObjectInterface<ITfKeyEventSink>>::as_interface_ref(self).to_owned();

            {
                let Ok(activation) = self.get_impl().activation.try_borrow() else {
                    return Err(E_UNEXPECTED.into());
                };
                if activation.is_some() {
                    return Err(E_UNEXPECTED.into());
                }
            }

            let popup = CandidatePopupWindow::new();
            if let Err(error) = &popup {
                record(Stage::PopupCreate, popup_activation_hresult(error));
            } else {
                record(Stage::PopupCreate, S_OK);
            }
            let mut popup = match popup {
                Ok(popup) => popup,
                Err(error) => return Err(popup_activation_hresult(&error).into()),
            };
            if let Err(error) = popup.set_theme(settings.theme) {
                record(Stage::PopupTheme, popup_activation_hresult(&error));
                return Err(popup_activation_hresult(&error).into());
            }
            let token = next_activation_token();
            let owner_thread = std::thread::current().id();
            let action_activation = Rc::clone(&self.get_impl().activation);
            let action_sink: Rc<dyn Fn(CandidatePopupAction)> = Rc::new(move |action| {
                dispatch_popup_action(&action_activation, token, owner_thread, action);
            });
            if let Err(error) = popup.set_action_sink(Some(action_sink)) {
                record(Stage::PopupActionSink, popup_activation_hresult(&error));
                return Err(popup_activation_hresult(&error).into());
            }
            if !register_thread_popup(token, popup) {
                return Err(E_UNEXPECTED.into());
            }

            let published = {
                let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                    close_thread_popup(token, owner_thread);
                    return Err(E_UNEXPECTED.into());
                };
                if activation.is_some() {
                    false
                } else {
                    *activation = Some(Activation {
                        thread_manager: thread_manager.clone(),
                        keystroke_manager: keystroke_manager.clone(),
                        client_id,
                        contexts: HashMap::new(),
                        token,
                        owner_thread,
                        focus_epoch: 0,
                        foreground: true,
                        shown_context: None,
                        processing: false,
                        settings,
                        lexicon,
                        rankings,
                        adaptive_ranking_enabled,
                        _learning_registration: learning
                            .as_ref()
                            .map(|_| ObjectRegistration::new()),
                        learning,
                        local_data_error,
                    });
                    true
                }
            };
            if !published {
                close_thread_popup(token, owner_thread);
                return Err(E_UNEXPECTED.into());
            }

            // SAFETY: Both interfaces belong to live COM objects. TSF retains
            // its own reference to `key_sink` after a successful advise.
            let advised =
                unsafe { keystroke_manager.AdviseKeyEventSink(client_id, &key_sink, TRUE) };
            record_result(Stage::AdviseKeySink, &advised);

            if let Err(error) = advised {
                let stale = {
                    let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                        close_thread_popup(token, owner_thread);
                        return Err(error);
                    };
                    let should_rollback = matches!(
                        activation.as_ref(),
                        Some(active) if active.token == token
                    );
                    if should_rollback {
                        activation.take()
                    } else {
                        None
                    }
                };
                drop(stale);
                close_thread_popup(token, owner_thread);
                return Err(error);
            }

            record(Stage::ActivationReady, S_OK);
            Ok(())
        }

        fn Deactivate(&self) -> Result<()> {
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            let active = {
                let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                    return Err(E_UNEXPECTED.into());
                };
                activation.take()
            };
            let Some(mut active) = active else {
                return Ok(());
            };

            // SAFETY: The keystroke manager was obtained from the thread manager
            // during activation and remains live in this state.
            let unadvised = unsafe {
                active
                    .keystroke_manager
                    .UnadviseKeyEventSink(active.client_id)
            };
            if let Err(error) = unadvised {
                let token = active.token;
                let owner_thread = active.owner_thread;
                let displaced = {
                    let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                        close_thread_popup(token, owner_thread);
                        return Err(error);
                    };
                    if activation.is_none() {
                        activation.replace(active)
                    } else {
                        Some(active)
                    }
                };
                let restored = displaced.is_none();
                drop(displaced);
                if !restored {
                    // A newer activation won the race, so the failed
                    // deactivation state cannot be restored. Release its
                    // window rather than leaking a thread-local popup.
                    close_thread_popup(token, owner_thread);
                }
                return Err(error);
            }

            close_thread_popup(active.token, active.owner_thread);
            active.shown_context = None;
            active.contexts.clear();
            if let Some(worker) = active.learning.as_mut() {
                if worker.shutdown().is_err() {
                    active.local_data_error = true;
                }
            }
            let local_data_error = active.local_data_error;
            drop(active);
            if local_data_error {
                show_local_data_error();
            }
            Ok(())
        }
    }

    impl ITfKeyEventSink_Impl for TextService_Impl {
        fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            // Every focus transition invalidates key work that started under
            // the previous focus state. Focus loss additionally hides the
            // popup and clears its context.
            let popup = {
                let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                    return Err(E_UNEXPECTED.into());
                };
                activation.as_mut().and_then(|active| {
                    active.focus_epoch = active.focus_epoch.wrapping_add(1);
                    active.foreground = foreground.as_bool();
                    if foreground.as_bool() {
                        None
                    } else {
                        active.shown_context = None;
                        Some((active.token, active.owner_thread))
                    }
                })
            };

            if let Some((token, owner_thread)) = popup {
                if let Some(mut popup) = take_thread_popup(token, owner_thread) {
                    popup.window.cancel_pending_actions();
                    popup.window.hide();

                    let still_owned = {
                        self.get_impl()
                            .activation
                            .try_borrow()
                            .ok()
                            .is_some_and(|activation| {
                                matches!(activation.as_ref(), Some(active) if active.token == token)
                            })
                    };
                    if still_owned {
                        return_thread_popup(token, owner_thread, popup);
                    } else {
                        // The activation ended while the window was leased;
                        // dropping destroys it outside the activation borrow.
                        drop(popup);
                    }
                }
            }

            Ok(())
        }

        fn OnTestKeyDown(
            &self,
            context: Option<&ITfContext>,
            wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            record(Stage::TestKeyDown, S_OK);
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            let Some(context) = context else {
                return Ok(FALSE);
            };
            let Some(event) = map_key_event(wparam) else {
                return Ok(FALSE);
            };
            let key = context_key(context);

            let (router, page_size, lexicon, rankings) = {
                let Ok(activation) = self.get_impl().activation.try_borrow() else {
                    return Ok(FALSE);
                };
                let Some(active) = activation.as_ref() else {
                    return Ok(FALSE);
                };
                if active.processing {
                    return Ok(FALSE);
                }
                (
                    active
                        .contexts
                        .get(&key)
                        .filter(|entry| !entry.composition_terminated())
                        .map(|entry| entry.router.clone()),
                    active.settings.candidate_page_size,
                    active.lexicon.clone(),
                    Arc::clone(&active.rankings),
                )
            };
            let mut router = match router {
                Some(router) => router,
                None => match EngineRouter::from_lexicon(lexicon, page_size) {
                    Ok(mut router) => {
                        router.set_candidate_width(Some(
                            crate::engine::paging::DEFAULT_CANDIDATE_WIDTH,
                        ));
                        router
                    }
                    Err(_) => return Ok(FALSE),
                },
            };
            if !router.is_composing() {
                router.set_ranking_orders(rankings);
            }

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
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            Ok(FALSE)
        }

        fn OnKeyDown(
            &self,
            context: Option<&ITfContext>,
            wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            record(Stage::KeyDown, S_OK);
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            let Some(context) = context else {
                return Ok(FALSE);
            };
            let Some(event) = map_key_event(wparam) else {
                return Ok(FALSE);
            };
            let key = context_key(context);

            let (
                entry,
                client_id,
                token,
                owner_thread,
                focus_epoch,
                thread_manager,
                settings,
                lexicon,
            ) = {
                let Ok(mut activation) = self.get_impl().activation.try_borrow_mut() else {
                    return Ok(FALSE);
                };
                let Some(active) = activation.as_mut() else {
                    return Ok(FALSE);
                };
                if active.processing {
                    return Ok(FALSE);
                }
                let entry = active.contexts.remove(&key);
                let client_id = active.client_id;
                let token = active.token;
                let owner_thread = active.owner_thread;
                let focus_epoch = active.focus_epoch;
                let thread_manager = active.thread_manager.clone();
                let settings = active.settings.clone();
                active.processing = true;
                (
                    entry,
                    client_id,
                    token,
                    owner_thread,
                    focus_epoch,
                    thread_manager,
                    settings,
                    active.lexicon.clone(),
                )
            };
            let _guard = ProcessingGuard {
                activation: Rc::clone(&self.get_impl().activation),
                token,
                owner_thread,
            };

            let was_existing = entry.is_some();
            let mut old_entry = entry.clone();
            let mut entry = match entry {
                Some(entry) => entry,
                None => match ContextEntry::new(context, &settings, &lexicon) {
                    Some(entry) => entry,
                    None => return Ok(FALSE),
                },
            };
            if entry.composition_terminated() {
                clear_invalid_popup_action(&self.get_impl().activation, token, owner_thread, key);
                let Ok(router) =
                    EngineRouter::from_lexicon(lexicon.clone(), settings.candidate_page_size)
                else {
                    return Ok(FALSE);
                };
                entry.router = router;
                entry
                    .router
                    .set_candidate_width(Some(crate::engine::paging::DEFAULT_CANDIDATE_WIDTH));
                entry.composition = None;
                entry.controller = CompositionController::new();
                entry.popup_model.hide();
                old_entry = Some(entry.clone());
            }

            sync_idle_rankings(&self.get_impl().activation, token, &mut entry.router);
            let update = match entry.router.process_key(event) {
                Ok(update) => update,
                Err(_) => {
                    if let Some(old_entry) = old_entry {
                        self.get_impl().restore_context(token, key, old_entry);
                    }
                    return Ok(FALSE);
                }
            };

            if !update.handled {
                if was_existing {
                    self.get_impl().restore_context(token, key, entry);
                }
                return Ok(FALSE);
            }

            let update_for_popup = update.clone();
            let outcome = request_key_edit_session(
                &entry.context,
                client_id,
                update,
                entry.composition.take(),
                entry.controller,
            );

            let Some(outcome) = outcome else {
                // The edit session did not report success. Restore the engine
                // and popup snapshots, but do not claim the TSF document is
                // unchanged: SetText may already have partially applied before
                // a later EndComposition failure.
                if let Some(old_entry) = old_entry {
                    self.get_impl().restore_context(token, key, old_entry);
                }
                return Ok(FALSE);
            };

            let target = EditTarget {
                promotion_claim: Rc::new(Cell::new(false)),
                activation: Rc::clone(&self.get_impl().activation),
                thread_manager,
                token,
                owner_thread,
                key,
                focus_epoch,
            };
            publish_successful_edit(&target, entry, &update_for_popup, outcome);

            Ok(TRUE)
        }

        fn OnKeyUp(
            &self,
            _context: Option<&ITfContext>,
            _wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
            Ok(FALSE)
        }

        fn OnPreservedKey(
            &self,
            _context: Option<&ITfContext>,
            _guid: *const GUID,
        ) -> Result<BOOL> {
            if !self.get_impl().owns_current_thread() {
                return Err(E_UNEXPECTED.into());
            }
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

    #[cfg(test)]
    mod native_tests {
        use super::*;
        use windows::core::{BSTR, VARIANT};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::TextServices::{
            CLSID_TF_ThreadMgr, ITfCompositionView, ITfContextOwnerCompositionServices,
            ITfDocumentMgr, TF_ES_READ, TF_POPF_ALL,
        };
        use windows::Win32::UI::TextServices::{
            ITfContextOwner, ITfContextOwner_Impl, ITfInputScope, ITfInputScope_Impl, InputScope,
            GUID_PROP_INPUTSCOPE, TS_STATUS,
        };

        #[implement(ITfInputScope)]
        struct TestScope(Vec<i32>);

        impl ITfInputScope_Impl for TestScope_Impl {
            fn GetInputScopes(&self, values: *mut *mut InputScope, count: *mut u32) -> Result<()> {
                unsafe {
                    *count = self.0.len() as u32;
                    *values = windows::Win32::System::Com::CoTaskMemAlloc(
                        self.0.len().max(1) * std::mem::size_of::<InputScope>(),
                    )
                    .cast();
                    if (*values).is_null() {
                        return Err(E_UNEXPECTED.into());
                    }
                    for (index, value) in self.0.iter().enumerate() {
                        (*values).add(index).write(InputScope(*value));
                    }
                }
                Ok(())
            }
            fn GetPhrase(&self, _: *mut *mut BSTR, _: *mut u32) -> Result<()> {
                Err(E_UNEXPECTED.into())
            }
            fn GetRegularExpression(&self) -> Result<BSTR> {
                Err(E_UNEXPECTED.into())
            }
            fn GetSRGS(&self) -> Result<BSTR> {
                Err(E_UNEXPECTED.into())
            }
            fn GetXML(&self) -> Result<BSTR> {
                Err(E_UNEXPECTED.into())
            }
        }

        #[implement(ITfContextOwner)]
        struct ScopeOwner {
            value: VARIANT,
            calls: Rc<Cell<u32>>,
            fail: bool,
        }

        impl ITfContextOwner_Impl for ScopeOwner_Impl {
            fn GetACPFromPoint(&self, _: *const POINT, _: u32) -> Result<i32> {
                Ok(0)
            }
            fn GetTextExt(
                &self,
                _: i32,
                _: i32,
                rect: *mut RECT,
                clipped: *mut BOOL,
            ) -> Result<()> {
                unsafe {
                    *rect = self.GetScreenExt()?;
                    *clipped = FALSE;
                }
                Ok(())
            }
            fn GetScreenExt(&self) -> Result<RECT> {
                Ok(RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 20,
                })
            }
            fn GetStatus(&self) -> Result<TS_STATUS> {
                Ok(TS_STATUS::default())
            }
            fn GetWnd(&self) -> Result<windows::Win32::Foundation::HWND> {
                Ok(windows::Win32::Foundation::HWND::default())
            }
            fn GetAttribute(&self, guid: *const GUID) -> Result<VARIANT> {
                if unsafe { *guid } != GUID_PROP_INPUTSCOPE {
                    return Err(E_INVALIDARG.into());
                }
                self.calls.set(self.calls.get() + 1);
                if self.fail {
                    return Err(E_UNEXPECTED.into());
                }
                Ok(self.value.clone())
            }
        }

        #[implement(ITfEditSession)]
        struct CompareScope {
            context: ITfContext,
            calls: Rc<Cell<u32>>,
        }
        impl ITfEditSession_Impl for CompareScope_Impl {
            fn DoEditSession(&self, cookie: u32) -> Result<()> {
                unsafe {
                    fn status<T>(stage: &str, result: &Result<T>, calls: u32) {
                        eprintln!(
                            "stage={stage} hr=0x{:08X} owner_calls={calls}",
                            result.as_ref().err().map_or(0, |e| e.code().0 as u32)
                        );
                    }
                    let mut selection = [TF_SELECTION::default()];
                    let mut fetched = 0;
                    let selected =
                        self.context
                            .GetSelection(cookie, u32::MAX, &mut selection, &mut fetched);
                    status("selection", &selected, self.calls.get());
                    let _range = std::mem::ManuallyDrop::take(&mut selection[0].range);
                    let start = self.context.GetStart(cookie);
                    status("start", &start, self.calls.get());
                    let range = start?;
                    range.SetText(cookie, 0, &[b'x' as u16])?;
                    let range = self.context.GetStart(cookie)?;
                    let end = self.context.GetEnd(cookie)?;
                    range.ShiftEndToRange(cookie, &end, TF_ANCHOR_END)?;
                    let property = self.context.GetProperty(&GUID_PROP_INPUTSCOPE);
                    status("document_property", &property, self.calls.get());
                    let document = property.and_then(|p| p.GetValue(cookie, &range));
                    status("document_value", &document, self.calls.get());
                    assert_eq!(self.calls.get(), 0, "document channel must not query owner");
                    let property = self.context.GetAppProperty(&GUID_PROP_INPUTSCOPE);
                    status("app_property", &property, self.calls.get());
                    let app = property.and_then(|p| p.GetValue(cookie, &range));
                    status("app_value", &app, self.calls.get());
                    let app = app?;
                    let scope: ITfInputScope = IUnknown::try_from(&app)?.cast()?;
                    assert!(
                        self.calls.get() > 0,
                        "application property must query owner"
                    );
                    let mut values = std::ptr::null_mut();
                    let mut count = 0;
                    scope.GetInputScopes(&mut values, &mut count)?;
                    assert_eq!(count, 1);
                    assert_eq!((*values).0, 0);
                    windows::Win32::System::Com::CoTaskMemFree(Some(values.cast()));
                }
                Ok(())
            }
        }

        #[test]
        fn native_scope_commit_publishes_rankings_and_worker() {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .unwrap();
            let _apartment = Apartment;
            let root = std::path::PathBuf::from(
                std::env::var_os("FLOWER_TEST_SCRATCH")
                    .expect("native learning test requires an explicitly isolated scratch root"),
            );
            assert!(root.is_absolute());
            for (index, (scopes, fail, allowed)) in [
                (Some(vec![0]), false, true),
                (Some(vec![57]), false, true),
                (Some(vec![58]), false, true),
                (Some(vec![31]), false, false),
                (Some(vec![61]), false, false),
                (Some(vec![0, 31]), false, false),
                (Some(vec![999]), false, false),
                (Some(vec![]), false, false),
                (None, false, false),
                (Some(vec![0]), true, false),
            ]
            .into_iter()
            .enumerate()
            {
                let value = scopes.map_or_else(VARIANT::default, |s| {
                    let scope: IUnknown = TestScope(s).into();
                    VARIANT::from(scope)
                });
                let calls = Rc::new(Cell::new(0));
                let owner: IUnknown = ScopeOwner {
                    value,
                    fail,
                    calls: calls.clone(),
                }
                .into();
                let host = NativeDocument::with_owner(Some(&owner)).unwrap();
                let path = root.join(format!(
                    "native-learning-{index}-{}.json",
                    std::process::id()
                ));
                assert!(!path.exists(), "test never reuses an existing store");
                let store = DictionaryStore::new(&path);
                let data = store.load().unwrap();
                let service: ITfTextInputProcessor = TextService::new().into();
                let implementation =
                    unsafe { <ITfTextInputProcessor as AsImpl<TextService>>::as_impl(&service) };
                *implementation.activation.borrow_mut() = Some(Activation {
                    thread_manager: host.manager.clone(),
                    keystroke_manager: host.manager.cast().unwrap(),
                    client_id: host.client,
                    contexts: HashMap::new(),
                    token: next_activation_token(),
                    owner_thread: std::thread::current().id(),
                    focus_epoch: 0,
                    foreground: true,
                    shown_context: None,
                    processing: false,
                    settings: Settings::default(),
                    lexicon: Lexicon::from_tsv("ying'she\t影射\t200\nying'she\t映射\t100").unwrap(),
                    rankings: Arc::new(Vec::new()),
                    adaptive_ranking_enabled: true,
                    learning: Some(LearningWorker::start(store, data.generation).unwrap()),
                    _learning_registration: None,
                    local_data_error: false,
                });
                let sink: ITfKeyEventSink = service.cast().unwrap();
                for key in b"YINGSHE2" {
                    send_key(&sink, &host.context, *key);
                }
                assert_eq!(read_document(&host, None), "映射");
                assert!(calls.get() > 0);
                let orders = implementation
                    .activation
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .rankings
                    .clone();
                assert_eq!(!orders.is_empty(), allowed, "scope case {index}");
                if allowed {
                    assert_eq!(orders[0].candidates[0].text, "映射");
                }
                for key in b"YINGSHE " {
                    send_key(&sink, &host.context, *key);
                }
                assert_eq!(
                    read_document(&host, None),
                    if allowed {
                        "映射映射"
                    } else {
                        "映射影射"
                    }
                );
                let mut active = implementation.activation.borrow_mut().take().unwrap();
                assert!(!active.local_data_error);
                active.learning.as_mut().unwrap().shutdown().unwrap();
                drop(active);
                let persisted = DictionaryStore::new(&path).load().unwrap();
                assert_eq!(!persisted.rankings.is_empty(), allowed);
                if allowed {
                    assert_eq!(persisted.rankings[0].candidates[0].text, "映射");
                }
            }
        }

        #[test]
        fn native_owner_application_scope_path() {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .unwrap();
            let _apartment = Apartment;
            let calls = Rc::new(Cell::new(0));
            let scope: IUnknown = TestScope(vec![0]).into();
            let owner: IUnknown = ScopeOwner {
                value: VARIANT::from(scope),
                calls: calls.clone(),
                fail: false,
            }
            .into();
            let host = NativeDocument::with_owner(Some(&owner)).unwrap();
            let session: ITfEditSession = CompareScope {
                context: host.context.clone(),
                calls,
            }
            .into();
            unsafe {
                host.context
                    .RequestEditSession(host.client, &session, TF_ES_SYNC | TF_ES_READWRITE)
            }
            .unwrap()
            .ok()
            .unwrap();
        }

        #[implement(ITfEditSession)]
        struct ReadDocument {
            context: ITfContext,
            terminate: Option<ActiveComposition>,
            text: Rc<RefCell<String>>,
        }

        impl ITfEditSession_Impl for ReadDocument_Impl {
            fn DoEditSession(&self, cookie: u32) -> Result<()> {
                if let Some(composition) = &self.terminate {
                    let owner: ITfContextOwnerCompositionServices = self.context.cast()?;
                    let view: ITfCompositionView = composition.handle.cast()?;
                    unsafe { owner.TerminateComposition(&view) }?;
                }
                let range = unsafe { self.context.GetStart(cookie) }?;
                let end = unsafe { self.context.GetEnd(cookie) }?;
                unsafe { range.ShiftEndToRange(cookie, &end, TF_ANCHOR_END) }?;
                let mut buffer = [0u16; 512];
                let mut fetched = 0;
                unsafe { range.GetText(cookie, 0, &mut buffer, &mut fetched) }?;
                *self.text.borrow_mut() = String::from_utf16(&buffer[..fetched as usize])
                    .map_err(|_| Error::from(E_UNEXPECTED))?;
                Ok(())
            }
        }

        fn read_document(host: &NativeDocument, terminate: Option<ActiveComposition>) -> String {
            let flags = if terminate.is_some() {
                TF_ES_READWRITE
            } else {
                TF_ES_READ
            };
            let text = Rc::new(RefCell::new(String::new()));
            let session: ITfEditSession = ReadDocument {
                context: host.context.clone(),
                terminate,
                text: Rc::clone(&text),
            }
            .into();
            unsafe {
                host.context
                    .RequestEditSession(host.client, &session, TF_ES_SYNC | flags)
            }
            .unwrap()
            .ok()
            .unwrap();
            let result = text.borrow().clone();
            result
        }

        fn send_key(sink: &ITfKeyEventSink, context: &ITfContext, key: u8) {
            assert!(
                unsafe { sink.OnTestKeyDown(context, WPARAM(key as usize), LPARAM(0)) }
                    .unwrap()
                    .as_bool()
            );
            assert!(
                unsafe { sink.OnKeyDown(context, WPARAM(key as usize), LPARAM(0)) }
                    .unwrap()
                    .as_bool(),
                "native TSF key must apply successfully"
            );
        }

        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }

        struct NativeDocument {
            manager: ITfThreadMgr,
            document: ITfDocumentMgr,
            context: ITfContext,
            client: u32,
            owner_cookie: Option<u32>,
        }

        impl NativeDocument {
            fn new() -> Result<Self> {
                Self::with_owner(None)
            }

            fn with_owner(owner: Option<&IUnknown>) -> Result<Self> {
                let manager: ITfThreadMgr =
                    unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER) }?;
                let client = unsafe { manager.Activate() }?;
                let document = unsafe { manager.CreateDocumentMgr() }?;
                let mut context = None;
                let mut cookie = 0;
                unsafe {
                    document.CreateContext(client, 0, owner, &mut context, &mut cookie)?;
                }
                let context = context.ok_or_else(|| Error::from(E_UNEXPECTED))?;
                let owner_cookie = if let Some(owner) = owner {
                    let source: windows::Win32::UI::TextServices::ITfSource = context.cast()?;
                    Some(unsafe { source.AdviseSink(&ITfContextOwner::IID, owner) }?)
                } else {
                    None
                };
                unsafe {
                    document.Push(&context)?;
                    manager.SetFocus(&document)?;
                }
                Ok(Self {
                    manager,
                    document,
                    context,
                    client,
                    owner_cookie,
                })
            }
        }

        impl Drop for NativeDocument {
            fn drop(&mut self) {
                unsafe {
                    if let Some(cookie) = self.owner_cookie {
                        if let Ok(source) = self
                            .context
                            .cast::<windows::Win32::UI::TextServices::ITfSource>()
                        {
                            let _ = source.UnadviseSink(cookie);
                        }
                    }
                    let _ = self.manager.SetFocus(None::<&ITfDocumentMgr>);
                    let _ = self.document.Pop(TF_POPF_ALL);
                    let _ = self.manager.Deactivate();
                }
            }
        }

        #[test]
        fn native_context_accepts_pinyin_in_write_edit_session() {
            // Only a new, TSF-owned empty document is used. This never reads a
            // user document, injects global keys, or changes profile selection.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .unwrap();
            let _apartment = Apartment;
            let host = NativeDocument::new().unwrap();
            let service: ITfTextInputProcessor = TextService::new().into();
            let implementation =
                unsafe { <ITfTextInputProcessor as AsImpl<TextService>>::as_impl(&service) };
            let token = next_activation_token();
            *implementation.activation.borrow_mut() = Some(Activation {
                thread_manager: host.manager.clone(),
                keystroke_manager: host.manager.cast().unwrap(),
                client_id: host.client,
                contexts: HashMap::new(),
                token,
                owner_thread: std::thread::current().id(),
                focus_epoch: 0,
                foreground: true,
                shown_context: None,
                processing: false,
                settings: Settings::default(),
                lexicon: Lexicon::builtin().unwrap(),
                rankings: Arc::new(Vec::new()),
                adaptive_ranking_enabled: false,
                learning: None,
                _learning_registration: None,
                local_data_error: false,
            });
            let sink: ITfKeyEventSink = service.cast().unwrap();
            let tested =
                unsafe { sink.OnTestKeyDown(&host.context, WPARAM(b'N' as usize), LPARAM(0)) };
            let handled =
                unsafe { sink.OnKeyDown(&host.context, WPARAM(b'N' as usize), LPARAM(0)) };
            assert!(
                tested.unwrap().as_bool(),
                "engine must accept a plain letter"
            );
            assert!(
                handled.unwrap().as_bool(),
                "native TSF edit must apply before eating the letter"
            );
            assert_eq!(read_document(&host, None), "n");
            for key in b"IHAO" {
                send_key(&sink, &host.context, *key);
            }
            assert_eq!(read_document(&host, None), "nihao");
            send_key(&sink, &host.context, b' ');
            assert_eq!(read_document(&host, None), "你好");
            for key in b"SHI" {
                send_key(&sink, &host.context, *key);
            }
            send_key(&sink, &host.context, 0x1B);
            assert_eq!(read_document(&host, None), "你好");
            for key in b"NI HAO " {
                send_key(&sink, &host.context, *key);
            }
            assert_eq!(
                read_document(&host, None),
                "你好你好",
                "commits must follow the caret"
            );
            for key in b"SHI" {
                send_key(&sink, &host.context, *key);
            }
            let entry = implementation
                .activation
                .borrow()
                .as_ref()
                .unwrap()
                .contexts
                .get(&context_key(&host.context))
                .unwrap()
                .clone();
            let revision = entry.popup_model.snapshot().unwrap().revision;
            let composition = entry.composition.unwrap();
            assert_eq!(
                read_document(&host, Some(composition.clone())),
                "你好你好shi"
            );
            assert!(
                composition.terminated.get(),
                "TSF must notify the real sink"
            );
            send_key(&sink, &host.context, b'N');
            assert!(
                implementation
                    .activation
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .contexts
                    .get(&context_key(&host.context))
                    .unwrap()
                    .popup_model
                    .snapshot()
                    .unwrap()
                    .revision
                    > revision,
                "old popup actions must stay invalid"
            );
            send_key(&sink, &host.context, b'I');
            send_key(&sink, &host.context, b' ');
            assert_eq!(read_document(&host, None), "你好你好shi你");
            // A minimal source isolates UTF-16 edit behavior from candidate rank.
            // The full dictionary's supplementary lookup is verified separately.
            implementation
                .activation
                .borrow_mut()
                .as_mut()
                .unwrap()
                .contexts
                .get_mut(&context_key(&host.context))
                .unwrap()
                .router = EngineRouter::from_lexicon_tsv("he\t𠀀\t1", 9).unwrap();
            for key in b"HE " {
                send_key(&sink, &host.context, *key);
            }
            assert_eq!(read_document(&host, None), "你好你好shi你𠀀");
            implementation
                .activation
                .borrow_mut()
                .as_mut()
                .unwrap()
                .contexts
                .get_mut(&context_key(&host.context))
                .unwrap()
                .router = EngineRouter::from_lexicon_tsv("ni\t你\t1000\nhao\t好\t1000", 9).unwrap();
            for key in b"NIHAO2" {
                send_key(&sink, &host.context, *key);
            }
            assert_eq!(read_document(&host, None), "你好你好shi你𠀀你hao");
            send_key(&sink, &host.context, b' ');
            assert_eq!(read_document(&host, None), "你好你好shi你𠀀你好");
            implementation
                .activation
                .borrow_mut()
                .as_mut()
                .unwrap()
                .contexts
                .get_mut(&context_key(&host.context))
                .unwrap()
                .router = EngineRouter::from_lexicon_tsv("ni'hao\t你好\t1000", 9).unwrap();
            for key in b"NH" {
                send_key(&sink, &host.context, *key);
            }
            assert_eq!(read_document(&host, None), "你好你好shi你𠀀你好nh");
            send_key(&sink, &host.context, b' ');
            assert_eq!(read_document(&host, None), "你好你好shi你𠀀你好你好");
            implementation.activation.borrow_mut().take();
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
        assert_eq!(TEXT_SERVICE_DESCRIPTION, "Flower");
    }
}
