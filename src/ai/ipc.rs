//! Local-only correction transport. The host must establish that the helper's
//! installation directory is protected; canonicalization is not an ACL check.

use super::{eligible_text, Request, Response};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;

#[derive(Default)]
struct Mailbox {
    pending: Option<(u64, Request)>,
    result: Option<Response>,
    generation: u64,
}

/// A single latest-only request and result slot. Public polling never waits for
/// a lock or performs file, pipe, process, or model operations.
pub struct AiClient {
    mailbox: Arc<Mutex<Mailbox>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AiClient {
    /// `helper` must come from the host's protected installation payload, never
    /// user configuration. This validates location syntax/existence, not ACLs.
    pub fn start(helper: PathBuf) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let helper = platform::helper_path(helper)?;
            let sid = platform::current_identity()?;
            let mailbox = Arc::new(Mutex::new(Mailbox::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let (m, s) = (mailbox.clone(), stop.clone());
            let worker = std::thread::Builder::new()
                .name("flower-ai-ipc".into())
                .spawn(move || platform::worker(helper, sid, m, s))
                .map_err(|_| "cannot start AI transport worker".to_string())?;
            Ok(Self {
                mailbox,
                stop,
                worker: Some(worker),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = helper;
            Err("offline AI IPC is unsupported on this platform".into())
        }
    }

    pub fn submit(&self, request: Request) -> bool {
        if self.stop.load(Ordering::Acquire)
            || request.text.len() > super::MAX_FRAME_BYTES
            || !eligible_text(&request.text)
        {
            return false;
        }
        let Ok(mut m) = self.mailbox.try_lock() else {
            return false;
        };
        m.generation = m.generation.wrapping_add(1);
        m.pending = Some((m.generation, request));
        m.result = None;
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        true
    }

    pub fn try_result(&self) -> Option<Response> {
        if self.stop.load(Ordering::Acquire) {
            return None;
        }
        self.mailbox.try_lock().ok()?.result.take()
    }

    /// Stop and join before the host releases its independent DLL lifetime pin.
    /// No detached thread can continue to execute transport/DLL code.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
        if let Ok(mut m) = self.mailbox.lock() {
            m.pending = None;
            m.result = None;
        }
    }
}

impl Drop for AiClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod mailbox_tests {
    use super::*;

    fn client() -> AiClient {
        AiClient {
            mailbox: Arc::new(Mutex::new(Mailbox::default())),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    #[test]
    fn latest_only_nonblocking_slots_and_repeatable_shutdown() {
        let mut client = client();
        for id in 0..100 {
            assert!(client.submit(Request {
                id,
                text: "今天心情很好".into(),
            }));
        }
        {
            let m = client.mailbox.lock().unwrap();
            assert_eq!(m.pending.as_ref().unwrap().1.id, 99);
            assert!(!client.submit(Request {
                id: 100,
                text: "今天心情很好".into(),
            }));
            assert!(client.try_result().is_none());
        }
        assert!(!client.submit(Request {
            id: 101,
            text: "private ASCII".into(),
        }));
        client.shutdown();
        client.shutdown();
        assert!(client.try_result().is_none());
        assert!(!client.submit(Request {
            id: 102,
            text: "今天心情很好".into(),
        }));
    }

    #[test]
    fn result_is_consumed_once_and_new_submission_clears_it() {
        let client = client();
        client.mailbox.lock().unwrap().result = Some(Response {
            id: 1,
            corrected: None,
        });
        assert_eq!(client.try_result().unwrap().id, 1);
        assert!(client.try_result().is_none());
        client.mailbox.lock().unwrap().result = Some(Response {
            id: 2,
            corrected: None,
        });
        assert!(client.submit(Request {
            id: 3,
            text: "今天心情很好".into(),
        }));
        assert!(client.try_result().is_none());
    }
}

/// Bind before invoking `correct`; callers can lazily load the model in this
/// process-owned closure. No model or host document state crosses the pipe.
pub fn serve(correct: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<(), String> {
    #[cfg(windows)]
    {
        platform::serve(correct)
    }
    #[cfg(not(windows))]
    {
        let _ = correct;
        Err("offline AI IPC is unsupported on this platform".into())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::ai::{valid_correction, MAX_FRAME_BYTES, RESULT_TIMEOUT_MS, SERVER_IDLE_SECONDS};
    use serde::{de::DeserializeOwned, Serialize};
    use sha2::{Digest, Sha256};
    use std::ffi::OsString;
    use std::os::windows::{ffi::OsStringExt, process::CommandExt};
    use std::path::Path;
    use std::time::{Duration, Instant};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Security::Authorization::*;
    use windows::Win32::Security::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::Pipes::*;
    use windows::Win32::System::Threading::*;
    use windows::Win32::System::IO::*;

    const POLL_MS: u32 = 10;
    const IO_LIMIT: Duration = Duration::from_millis(RESULT_TIMEOUT_MS);

    struct Handle(HANDLE);
    impl Handle {
        fn new(h: HANDLE) -> Result<Self, String> {
            if h.is_invalid() {
                Err("cannot create AI transport handle".into())
            } else {
                Ok(Self(h))
            }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this wrapper uniquely owns a valid kernel handle.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct LocalAllocation(HLOCAL);
    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            // SAFETY: allocations come from Convert* security APIs.
            unsafe {
                LocalFree(self.0);
            }
        }
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn helper_path(path: PathBuf) -> Result<PathBuf, String> {
        let spelling = path.to_string_lossy();
        if !path.is_absolute()
            || spelling.starts_with(r"\\") && !spelling.starts_with(r"\\?\")
            || spelling.starts_with(r"\\?\UNC\")
        {
            return Err("AI helper must be an absolute local installation path".into());
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| "AI helper is missing".to_string())?;
        if !canonical.is_file()
            || !canonical
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("flower-settings.exe"))
            || !matches!(
                canonical.components().next(),
                Some(std::path::Component::Prefix(prefix))
                    if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))
            )
        {
            return Err("AI helper is not the fixed settings executable".into());
        }
        Ok(canonical)
    }

    fn identity(process: HANDLE) -> Result<String, String> {
        // SAFETY: token output, aligned storage and SID live until conversion.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token)
                .map_err(|_| "cannot query AI process token".to_string())?;
            let token = Handle::new(token)?;
            let mut elevation = TOKEN_ELEVATION::default();
            let mut size = 0;
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            )
            .map_err(|_| "cannot query AI token elevation".to_string())?;
            if elevation.TokenIsElevated != 0 {
                return Err("elevated AI processes are not supported".into());
            }
            let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
            if size == 0 || size > 65536 {
                return Err("invalid AI token user size".into());
            }
            let mut storage = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            GetTokenInformation(
                token.0,
                TokenUser,
                Some(storage.as_mut_ptr().cast()),
                size,
                &mut size,
            )
            .map_err(|_| "cannot query AI token user".to_string())?;
            let user = &*storage.as_ptr().cast::<TOKEN_USER>();
            let mut sid = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid)
                .map_err(|_| "cannot encode AI user SID".to_string())?;
            let _allocation = LocalAllocation(HLOCAL(sid.0.cast()));
            sid.to_string().map_err(|_| "invalid AI user SID".into())
        }
    }

    pub(super) fn current_identity() -> Result<String, String> {
        // SAFETY: GetCurrentProcess returns a borrowed pseudo handle.
        identity(unsafe { GetCurrentProcess() })
    }

    fn endpoint(helper: &Path, sid: &str) -> Vec<u16> {
        let mut hash = Sha256::new();
        // Canonical parent + fixed executable + package version separates
        // installations and versions, independent of cwd or user environment.
        for unit in wide(helper.as_os_str()) {
            hash.update(unit.to_le_bytes());
        }
        hash.update(env!("CARGO_PKG_VERSION").as_bytes());
        let digest = format!("{:x}", hash.finalize());
        wide(std::ffi::OsStr::new(&format!(
            r"\\.\pipe\Flower.AI.{sid}.{digest}"
        )))
    }

    fn peer(pipe: HANDLE, helper: Option<&Path>, sid: &str) -> Result<(), String> {
        // SAFETY: pipe is connected; process is retained during image/token checks.
        unsafe {
            let mut pid = 0;
            if helper.is_some() {
                GetNamedPipeServerProcessId(pipe, &mut pid)
            } else {
                GetNamedPipeClientProcessId(pipe, &mut pid)
            }
            .map_err(|_| "cannot identify AI pipe peer".to_string())?;
            let process = Handle::new(
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                    .map_err(|_| "cannot inspect AI pipe peer".to_string())?,
            )?;
            if identity(process.0)? != sid {
                return Err("AI pipe peer has a different user".into());
            }
            if let Some(expected) = helper {
                let mut image = vec![0u16; 32768];
                let mut len = image.len() as u32;
                QueryFullProcessImageNameW(
                    process.0,
                    PROCESS_NAME_WIN32,
                    PWSTR(image.as_mut_ptr()),
                    &mut len,
                )
                .map_err(|_| "cannot inspect AI server image".to_string())?;
                let actual = PathBuf::from(OsString::from_wide(&image[..len as usize]))
                    .canonicalize()
                    .map_err(|_| "cannot resolve AI server image".to_string())?;
                if actual != expected {
                    return Err("AI server image does not match installation".into());
                }
            }
            Ok(())
        }
    }

    /// Every operation owns its event and OVERLAPPED until completion, including
    /// cancellation completion. Never return a buffer still borrowed by the OS.
    fn operation(
        pipe: HANDLE,
        stop: &AtomicBool,
        deadline: Instant,
        begin: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
        connecting: bool,
    ) -> Result<u32, String> {
        // SAFETY: event/OVERLAPPED remain live until I/O completes or is drained.
        unsafe {
            let event = Handle::new(
                CreateEventW(None, true, false, PCWSTR::null())
                    .map_err(|_| "cannot create AI I/O event".to_string())?,
            )?;
            let mut ov = OVERLAPPED {
                hEvent: event.0,
                ..Default::default()
            };
            match begin(&mut ov) {
                Ok(()) => {}
                Err(e) if connecting && e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                    return Ok(0);
                }
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                Err(_) => return Err("AI pipe operation failed".into()),
            }
            loop {
                let mut count = 0;
                match GetOverlappedResult(pipe, &ov, &mut count, false) {
                    Ok(()) => return Ok(count),
                    Err(e) if e.code() == ERROR_IO_INCOMPLETE.to_hresult() => {}
                    Err(_) => return Err("AI pipe completion failed".into()),
                }
                if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                    let _ = CancelIoEx(pipe, Some(&ov));
                    // CancelIoEx only requests cancellation. Drain the local
                    // named-pipe operation before freeing its buffer/OVERLAPPED.
                    let _ = GetOverlappedResult(pipe, &ov, &mut count, true);
                    return Err("AI pipe stopped or timed out".into());
                }
                if WaitForSingleObject(event.0, POLL_MS) == WAIT_FAILED {
                    let _ = CancelIoEx(pipe, Some(&ov));
                    let _ = GetOverlappedResult(pipe, &ov, &mut count, true);
                    return Err("AI pipe event wait failed".into());
                }
            }
        }
    }

    fn read_exact(
        pipe: HANDLE,
        mut bytes: &mut [u8],
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        while !bytes.is_empty() {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("AI read expired".into());
            }
            let count = operation(
                pipe,
                stop,
                deadline,
                // SAFETY: bytes remain borrowed until operation drains.
                |ov| unsafe { ReadFile(pipe, Some(bytes), None, Some(ov)) },
                false,
            )? as usize;
            if count == 0 || count > bytes.len() {
                return Err("AI pipe EOF".into());
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }

    fn read_frame<T: DeserializeOwned>(
        pipe: HANDLE,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<T, String> {
        let mut prefix = [0; 4];
        read_exact(pipe, &mut prefix, stop, deadline)?;
        let len = u32::from_le_bytes(prefix) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Err("invalid AI frame length".into());
        }
        let mut body = vec![0; len];
        read_exact(pipe, &mut body, stop, deadline)?;
        serde_json::from_slice(&body).map_err(|_| "invalid AI frame JSON".into())
    }

    fn write_frame(
        pipe: HANDLE,
        value: &impl Serialize,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        let body = serde_json::to_vec(value).map_err(|_| "cannot encode AI frame".to_string())?;
        if body.is_empty() || body.len() > MAX_FRAME_BYTES {
            return Err("AI frame exceeds bound".into());
        }
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend(body);
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("AI write expired".into());
            }
            let count = operation(
                pipe,
                stop,
                deadline,
                // SAFETY: remaining remains borrowed until operation drains.
                |ov| unsafe { WriteFile(pipe, Some(remaining), None, Some(ov)) },
                false,
            )? as usize;
            if count == 0 || count > remaining.len() {
                return Err("AI pipe short write".into());
            }
            remaining = &remaining[count..];
        }
        Ok(())
    }

    fn open(name: &[u16]) -> windows::core::Result<HANDLE> {
        // SAFETY: nul-terminated local pipe name; non-inheritable handle.
        unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        }
    }

    fn connect(
        helper: &Path,
        sid: &str,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<Handle, String> {
        let name = endpoint(helper, sid);
        let mut spawned = false;
        loop {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("AI connection expired".into());
            }
            match open(&name) {
                Ok(h) => {
                    let pipe = Handle::new(h)?;
                    // No request is sent to an unverified process.
                    peer(pipe.0, Some(helper), sid)?;
                    return Ok(pipe);
                }
                Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() && !spawned => {
                    std::process::Command::new(helper)
                        .arg("--ai-server")
                        .creation_flags(CREATE_NO_WINDOW.0)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .map_err(|_| "cannot launch AI broker".to_string())?;
                    spawned = true;
                }
                Err(e)
                    if e.code() == ERROR_PIPE_BUSY.to_hresult()
                        || e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {}
                Err(_) => return Err("AI connection rejected".into()),
            }
            std::thread::park_timeout(Duration::from_millis(POLL_MS as u64));
        }
    }

    pub(super) fn worker(
        helper: PathBuf,
        sid: String,
        mailbox: Arc<Mutex<Mailbox>>,
        stop: Arc<AtomicBool>,
    ) {
        while !stop.load(Ordering::Acquire) {
            let next = match mailbox.lock() {
                Ok(mut m) => m.pending.take(),
                Err(_) => return,
            };
            let Some((generation, request)) = next else {
                std::thread::park_timeout(Duration::from_millis(POLL_MS as u64));
                continue;
            };
            let deadline = Instant::now() + IO_LIMIT;
            let response = (|| {
                let pipe = connect(&helper, &sid, &stop, deadline)?;
                write_frame(pipe.0, &request, &stop, deadline)?;
                let response: Response = read_frame(pipe.0, &stop, deadline)?;
                checked_response(&request, response)
            })();
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                continue;
            }
            if let Ok(response) = response {
                if let Ok(mut m) = mailbox.lock() {
                    if m.generation == generation {
                        m.result = Some(response);
                    }
                }
            }
        }
    }

    fn checked_response(request: &Request, response: Response) -> Result<Response, String> {
        if response.id != request.id
            || response
                .corrected
                .as_ref()
                .is_some_and(|s| !valid_correction(&request.text, s))
        {
            return Err("invalid AI response identity or correction".into());
        }
        Ok(response)
    }

    fn bind(name: &[u16], sid: &str) -> Result<Handle, String> {
        let sddl = wide(std::ffi::OsStr::new(&format!(
            "D:P(A;;GA;;;SY)(A;;GA;;;{sid})"
        )));
        // SAFETY: descriptor allocation outlives CreateNamedPipe; no existing
        // object's ACL is changed. FIRST_PIPE_INSTANCE prevents name takeover.
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|_| "cannot construct AI pipe DACL".to_string())?;
            let _allocation = LocalAllocation(HLOCAL(descriptor.0));
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            Handle::new(CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_FRAME_BYTES as u32 + 4,
                MAX_FRAME_BYTES as u32 + 4,
                RESULT_TIMEOUT_MS as u32,
                Some(&attributes),
            ))
        }
    }

    pub(super) fn serve(
        correct: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<(), String> {
        let helper = helper_path(
            std::env::current_exe().map_err(|_| "cannot resolve AI broker image".to_string())?,
        )?;
        let sid = current_identity()?;
        let pipe = bind(&endpoint(&helper, &sid), &sid)?;
        run(
            pipe,
            &sid,
            Duration::from_secs(SERVER_IDLE_SECONDS),
            correct,
        )
    }

    fn run(
        pipe: Handle,
        sid: &str,
        idle_for: Duration,
        mut correct: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<(), String> {
        let stop = AtomicBool::new(false);
        let mut idle = Instant::now() + idle_for;
        loop {
            if Instant::now() >= idle {
                return Ok(());
            }
            let connected = operation(
                pipe.0,
                &stop,
                idle,
                // SAFETY: bound pipe and live OVERLAPPED.
                |ov| unsafe { ConnectNamedPipe(pipe.0, Some(ov)) },
                true,
            );
            if connected.is_err() {
                if Instant::now() >= idle {
                    return Ok(());
                }
                return Err("AI broker accept failed".into());
            }
            let request: Result<Request, String> = (|| {
                peer(pipe.0, None, sid)?;
                let request: Request =
                    read_frame(pipe.0, &stop, idle.min(Instant::now() + IO_LIMIT))?;
                if !eligible_text(&request.text) {
                    return Err("ineligible AI request".into());
                }
                Ok(request)
            })();
            let request = match request {
                Ok(request) => request,
                Err(_) => {
                    unsafe {
                        let _ = DisconnectNamedPipe(pipe.0);
                    }
                    continue;
                }
            };
            idle = Instant::now() + idle_for;
            let corrected = match correct(&request.text) {
                Ok(value) => value,
                Err(_) => return Err("AI correction initialization or inference failed".into()),
            };
            if corrected
                .as_ref()
                .is_some_and(|s| !valid_correction(&request.text, s))
            {
                return Err("invalid model correction".into());
            }
            let _ = (|| {
                write_frame(
                    pipe.0,
                    &Response {
                        id: request.id,
                        corrected,
                    },
                    &stop,
                    Instant::now() + IO_LIMIT,
                )?;
                // One request per connection. Wait for client EOF after it
                // consumes the response; disconnecting immediately can discard
                // unread bytes. No extra wire message or unbounded flush.
                let mut unexpected = [0];
                read_exact(pipe.0, &mut unexpected, &stop, Instant::now() + IO_LIMIT)
            })();
            // Do not FlushFileBuffers: a malicious/stalled reader must not
            // block broker teardown. The client reads before closing its end.
            unsafe {
                let _ = DisconnectNamedPipe(pipe.0);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::AtomicU64;
        use std::sync::mpsc;

        static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);

        fn test_name() -> Vec<u16> {
            wide(std::ffi::OsStr::new(&format!(
                r"\\.\pipe\Flower.AI.Test.{}.{}",
                std::process::id(),
                NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
            )))
        }

        fn accept(pipe: HANDLE, stop: &AtomicBool, deadline: Instant) -> Result<u32, String> {
            operation(
                pipe,
                stop,
                deadline,
                |ov| unsafe { ConnectNamedPipe(pipe, Some(ov)) },
                true,
            )
        }

        #[test]
        fn real_pending_accept_is_cancelled_and_worker_joined() {
            let sid = current_identity().expect("requires a non-elevated test token");
            let name = test_name();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let exited = Arc::new(AtomicBool::new(false));
            let thread_exited = exited.clone();
            let (ready, receive) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let pipe = bind(&name, &sid).unwrap();
                ready.send(()).unwrap();
                assert!(accept(
                    pipe.0,
                    &thread_stop,
                    Instant::now() + Duration::from_secs(60)
                )
                .is_err());
                thread_exited.store(true, Ordering::Release);
            });
            receive.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut client = AiClient {
                mailbox: Arc::new(Mutex::new(Mailbox::default())),
                stop,
                worker: Some(worker),
            };
            let start = Instant::now();
            client.shutdown();
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(exited.load(Ordering::Acquire));
            assert!(client.worker.is_none());
            client.shutdown();
        }

        // Each case owns a fresh, process-unique local pipe and both ends.
        fn frame_case(bytes: Vec<u8>, stall: bool) -> String {
            let sid = current_identity().expect("requires a non-elevated test token");
            let name = test_name();
            let client_name = name.clone();
            let (ready, receive) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let pipe = bind(&name, &sid).unwrap();
                assert!(bind(&name, &sid).is_err(), "first-instance protection");
                ready.send(()).unwrap();
                let stop = AtomicBool::new(false);
                accept(pipe.0, &stop, Instant::now() + Duration::from_secs(3)).unwrap();
                // Real token/SID check, not a mocked security decision.
                peer(pipe.0, None, &sid).unwrap();
                read_frame::<Request>(pipe.0, &stop, Instant::now() + Duration::from_millis(100))
                    .unwrap_err()
            });
            receive.recv_timeout(Duration::from_secs(3)).unwrap();
            let pipe = Handle::new(open(&client_name).unwrap()).unwrap();
            let stop = AtomicBool::new(false);
            if !bytes.is_empty() {
                operation(
                    pipe.0,
                    &stop,
                    Instant::now() + IO_LIMIT,
                    |ov| unsafe { WriteFile(pipe.0, Some(&bytes), None, Some(ov)) },
                    false,
                )
                .unwrap();
            }
            if stall {
                // Joining the timed-out reader proves it is not waiting on
                // this still-open client indefinitely.
                let error = server.join().unwrap();
                drop(pipe);
                error
            } else {
                drop(pipe);
                server.join().unwrap()
            }
        }

        #[test]
        fn frames_reject_zero_oversize_eof_and_stalled_body() {
            assert!(frame_case(0u32.to_le_bytes().to_vec(), true).contains("length"));
            assert!(frame_case(4097u32.to_le_bytes().to_vec(), true).contains("length"));
            assert!(frame_case(vec![], false).contains("failed"));
            assert!(frame_case(10u32.to_le_bytes().to_vec(), true).contains("timed out"));
        }

        #[test]
        fn server_image_mismatch_is_rejected_before_text() {
            let sid = current_identity().expect("requires a non-elevated test token");
            let name = test_name();
            let client_name = name.clone();
            let server_sid = sid.clone();
            let (ready, receive) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let pipe = bind(&name, &server_sid).unwrap();
                ready.send(()).unwrap();
                let stop = AtomicBool::new(false);
                accept(pipe.0, &stop, Instant::now() + Duration::from_secs(3)).unwrap();
                let mut byte = [0];
                assert!(read_exact(
                    pipe.0,
                    &mut byte,
                    &stop,
                    Instant::now() + Duration::from_secs(3)
                )
                .is_err());
            });
            receive.recv_timeout(Duration::from_secs(3)).unwrap();
            let pipe = Handle::new(open(&client_name).unwrap()).unwrap();
            let actual = std::env::current_exe().unwrap().canonicalize().unwrap();
            peer(pipe.0, Some(&actual), &sid).unwrap();
            assert!(peer(pipe.0, Some(Path::new(r"C:\not-the-server.exe")), &sid).is_err());
            assert!(peer(pipe.0, Some(&actual), "S-1-0-0").is_err());
            drop(pipe);
            server.join().unwrap();
        }

        #[test]
        fn broker_idle_exits_without_initializing_model() {
            let sid = current_identity().expect("requires a non-elevated test token");
            let pipe = bind(&test_name(), &sid).unwrap();
            let start = Instant::now();
            run(pipe, &sid, Duration::from_millis(100), |_| {
                panic!("idle broker must never initialize model")
            })
            .unwrap();
            assert!(start.elapsed() >= Duration::from_millis(100));
            assert!(start.elapsed() < Duration::from_secs(2));
        }

        #[test]
        fn response_validation_rejects_id_mismatch_and_unsafe_corrections() {
            let request = Request {
                id: 7,
                text: "今天新情很好".into(),
            };
            for response in [
                Response {
                    id: 8,
                    corrected: None,
                },
                Response {
                    id: 7,
                    corrected: Some("今天新情很好".into()),
                },
                Response {
                    id: 7,
                    corrected: Some("天气实在不好".into()),
                },
                Response {
                    id: 7,
                    corrected: Some("private ASCII".into()),
                },
            ] {
                assert!(checked_response(&request, response).is_err());
            }
            assert!(checked_response(
                &request,
                Response {
                    id: 7,
                    corrected: None
                }
            )
            .is_ok());
        }

        #[test]
        fn actual_pipe_dacl_is_protected_and_grants_only_user_and_system() {
            let sid = current_identity().expect("requires a non-elevated test token");
            let pipe = bind(&test_name(), &sid).unwrap();
            // Read only this test's newly created object's descriptor.
            unsafe {
                let mut descriptor = PSECURITY_DESCRIPTOR::default();
                let mut acl = std::ptr::null_mut();
                GetSecurityInfo(
                    pipe.0,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(&mut acl),
                    None,
                    Some(&mut descriptor),
                )
                .ok()
                .unwrap();
                let _allocation = LocalAllocation(HLOCAL(descriptor.0));
                let mut control = 0;
                let mut revision = 0;
                GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).unwrap();
                assert_ne!(control & SE_DACL_PROTECTED.0, 0);
                assert!(!acl.is_null());
                assert_eq!((*acl).AceCount, 2);
                let mut granted = Vec::new();
                for index in 0..2 {
                    let mut ace = std::ptr::null_mut();
                    GetAce(acl, index, &mut ace).unwrap();
                    let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                    assert_eq!(ace.Header.AceType, 0, "ACCESS_ALLOWED_ACE_TYPE");
                    let mut encoded = PWSTR::null();
                    ConvertSidToStringSidW(
                        PSID((&ace.SidStart as *const u32).cast_mut().cast()),
                        &mut encoded,
                    )
                    .unwrap();
                    let _sid_allocation = LocalAllocation(HLOCAL(encoded.0.cast()));
                    granted.push(encoded.to_string().unwrap());
                }
                granted.sort();
                let mut expected = vec![sid, "S-1-5-18".into()];
                expected.sort();
                assert_eq!(granted, expected);
            }
        }

        #[test]
        fn broker_rejects_ineligible_input_and_exits_on_model_error() {
            for eligible in [false, true] {
                let sid = current_identity().expect("requires a non-elevated test token");
                let name = test_name();
                let client_name = name.clone();
                let (ready, receive) = mpsc::channel();
                let server = std::thread::spawn(move || {
                    let pipe = bind(&name, &sid).unwrap();
                    ready.send(()).unwrap();
                    let mut calls = 0;
                    let result = run(pipe, &sid, Duration::from_millis(200), |_| {
                        calls += 1;
                        Err("test-only initialization failure".into())
                    });
                    assert_eq!(calls, usize::from(eligible));
                    if eligible {
                        assert!(result.is_err(), "model error must terminate broker");
                    } else {
                        result.unwrap();
                    }
                });
                receive.recv_timeout(Duration::from_secs(3)).unwrap();
                let pipe = Handle::new(open(&client_name).unwrap()).unwrap();
                let stop = AtomicBool::new(false);
                write_frame(
                    pipe.0,
                    &Request {
                        id: 1,
                        text: if eligible {
                            "今天心情很好"
                        } else {
                            "ASCII"
                        }
                        .into(),
                    },
                    &stop,
                    Instant::now() + IO_LIMIT,
                )
                .unwrap();
                assert!(read_frame::<Response>(pipe.0, &stop, Instant::now() + IO_LIMIT).is_err());
                drop(pipe);
                server.join().unwrap();
            }
        }

        #[test]
        fn actual_worker_drops_stale_result_and_stops_pending_read() {
            let sid = current_identity().expect("requires a non-elevated test token");
            // This binary is an owned test process, not the production helper.
            // Only this test binds its private test-executable endpoint.
            let helper = std::env::current_exe().unwrap().canonicalize().unwrap();
            let name = endpoint(&helper, &sid);
            let server_sid = sid.clone();
            let (events, receive) = mpsc::channel();
            let (release, wait_release) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let pipe = bind(&name, &server_sid).unwrap();
                let stop = AtomicBool::new(false);
                events.send(0).unwrap();
                for expected in 1..=3 {
                    accept(pipe.0, &stop, Instant::now() + Duration::from_secs(3)).unwrap();
                    let request: Request =
                        read_frame(pipe.0, &stop, Instant::now() + IO_LIMIT).unwrap();
                    assert_eq!(request.id, expected);
                    events.send(expected).unwrap();
                    if expected == 1 {
                        wait_release.recv_timeout(Duration::from_secs(3)).unwrap();
                    }
                    if expected == 2 {
                        // Artificial slow response, not a real model cold start.
                        std::thread::sleep(Duration::from_millis(1200));
                    }
                    if expected != 3 {
                        write_frame(
                            pipe.0,
                            &Response {
                                id: request.id,
                                corrected: Some("今天心情很好".into()),
                            },
                            &stop,
                            Instant::now() + IO_LIMIT,
                        )
                        .unwrap();
                    }
                    let mut byte = [0];
                    assert!(read_exact(
                        pipe.0,
                        &mut byte,
                        &stop,
                        Instant::now() + Duration::from_secs(3)
                    )
                    .is_err());
                    unsafe {
                        DisconnectNamedPipe(pipe.0).unwrap();
                    }
                }
            });
            assert_eq!(receive.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
            let mailbox = Arc::new(Mutex::new(Mailbox::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let (m, s) = (mailbox.clone(), stop.clone());
            let worker = std::thread::spawn(move || worker(helper, sid, m, s));
            let mut client = AiClient {
                mailbox,
                stop,
                worker: Some(worker),
            };
            let submit = |id| {
                let deadline = Instant::now() + IO_LIMIT;
                while !client.submit(Request {
                    id,
                    text: "今天新情很好".into(),
                }) {
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
            };
            submit(1);
            assert_eq!(receive.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
            let slow_response_start = Instant::now();
            submit(2);
            release.send(()).unwrap();
            assert_eq!(receive.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(result) = client.try_result() {
                    assert_eq!(result.id, 2, "old response must never replace latest");
                    assert_eq!(result.corrected.as_deref(), Some("今天心情很好"));
                    assert!(
                        slow_response_start.elapsed() > Duration::from_millis(1000),
                        "slow response must outlive the former one-second deadline"
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "worker did not return response");
                std::thread::yield_now();
            }
            // The peer accepts but deliberately withholds this response.
            submit(3);
            assert_eq!(receive.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
            let start = Instant::now();
            client.shutdown();
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(client.worker.is_none());
            assert!(client.try_result().is_none());
            server.join().unwrap();
        }
    }
}
