#![cfg_attr(not(target_os = "windows"), forbid(unsafe_code))]
//! Windows Text Services Framework boundary for Flower.
//!
//! The TSF/COM module is compiled only for Windows. It exports the in-process
//! server entry points, a class factory for the Flower text service, and a
//! text service that advises a pass-through key event sink while active.
//! Composition, candidate rendering, and per-context engine sessions remain
//! follow-up work.

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
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard};

    use windows::core::{implement, IUnknown, Interface, Result, GUID, HRESULT};
    use windows::Win32::Foundation::{
        BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER,
        E_UNEXPECTED, FALSE, LPARAM, S_FALSE, S_OK, TRUE, WPARAM,
    };
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows::Win32::UI::TextServices::{
        ITfContext, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfTextInputProcessor,
        ITfTextInputProcessor_Impl, ITfThreadMgr,
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

    struct Activation {
        keystroke_manager: ITfKeystrokeMgr,
        client_id: u32,
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
    }

    impl ITfTextInputProcessor_Impl for TextService_Impl {
        fn Activate(&self, thread_manager: Option<&ITfThreadMgr>, client_id: u32) -> Result<()> {
            let Some(thread_manager) = thread_manager else {
                return Err(E_INVALIDARG.into());
            };

            let mut activation = self.get_impl().lock_activation();
            if activation.is_some() {
                return Err(E_UNEXPECTED.into());
            }

            let keystroke_manager: ITfKeystrokeMgr = thread_manager.cast()?;
            let key_sink: ITfKeyEventSink =
                <Self as ComObjectInterface<ITfKeyEventSink>>::as_interface_ref(self).to_owned();

            // SAFETY: Both interfaces belong to live COM objects. TSF retains
            // its own reference to `key_sink` after a successful advise.
            unsafe {
                keystroke_manager.AdviseKeyEventSink(client_id, &key_sink, TRUE)?;
            }

            *activation = Some(Activation {
                keystroke_manager,
                client_id,
            });

            Ok(())
        }

        fn Deactivate(&self) -> Result<()> {
            let mut activation = self.get_impl().lock_activation();
            let Some(active) = activation.as_ref() else {
                return Ok(());
            };

            // SAFETY: The keystroke manager was obtained from the thread manager
            // during activation and remains live in this state.
            unsafe {
                active
                    .keystroke_manager
                    .UnadviseKeyEventSink(active.client_id)?;
            }

            let previous = activation.take();
            drop(activation);
            drop(previous);

            Ok(())
        }
    }

    impl ITfKeyEventSink_Impl for TextService_Impl {
        fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
            Ok(())
        }

        fn OnTestKeyDown(
            &self,
            _context: Option<&ITfContext>,
            _wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            Ok(FALSE)
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
            _context: Option<&ITfContext>,
            _wparam: WPARAM,
            _lparam: LPARAM,
        ) -> Result<BOOL> {
            Ok(FALSE)
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
