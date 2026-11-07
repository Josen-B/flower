#![cfg(windows)]
//! Explicit packaged-helper probe: owns/reaps its broker and sends only fixed fixtures.
use flower::ai::{ipc::AiClient, Request};
use std::os::windows::ffi::OsStrExt;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows::{core::PCWSTR, Win32::System::Pipes::WaitNamedPipeW};
struct Broker(Child);
impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
#[ignore = "requires explicit verified packaged helper and pipe; never installs or downloads"]
fn real_packaged_ai_client_accepts_cold_result_and_joins() {
    let helper = PathBuf::from(std::env::var_os("FLOWER_AI_TEST_HELPER").expect("explicit helper"));
    assert!(helper.is_absolute());
    let pipe = std::env::var_os("FLOWER_AI_TEST_PIPE").expect("explicit owned broker pipe");
    let wide: Vec<u16> = pipe.encode_wide().chain(Some(0)).collect();
    let mut broker = Broker(
        Command::new(&helper)
            .arg("--ai-server")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let ready = Instant::now() + Duration::from_secs(5);
    // Wait only for our broker to bind; the model remains unloaded until request.
    while !unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), 50) }.as_bool() {
        assert!(
            broker.0.try_wait().unwrap().is_none(),
            "owned broker exited before bind"
        );
        assert!(Instant::now() < ready, "owned broker did not bind");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut client = AiClient::start(helper).unwrap();
    let mut requests = Vec::new();
    for raw in [
        "xuanzaizainalishangban",
        "xainzaizainalishangban",
        "xinzaizainalishangban",
        "xiaanzaizainalishangban",
        "xianzaizainalishangban",
    ] {
        use flower::engine::{EngineRouter, Key, KeyEvent};
        let mut router = EngineRouter::new().unwrap();
        router.set_candidate_width(Some(flower::engine::paging::DEFAULT_CANDIDATE_WIDTH));
        for c in raw.chars() {
            router.process_key(KeyEvent::plain(Key::Char(c))).unwrap();
        }
        let input = router
            .ai_input()
            .expect("actual complete builtin candidate");
        let correct = raw == "xianzaizainalishangban";
        let reading = if correct || raw == "xinzaizainalishangban" {
            ""
        } else {
            "xian'zai'zai'na'li'shang'ban"
        };
        requests.push((
            Request {
                id: requests.len() as u64 + 1,
                text: input.source,
                raw: input.raw,
                reading: input.reading,
            },
            (!correct).then_some("现在在哪里上班"),
            reading,
        ));
    }
    for (id, source, raw, reading, expected, expected_reading) in [
        (1, "请问现在那里上班", "", "", Some("请问现在哪里上班"), ""),
        (
            2,
            "选在在哪里上班",
            "xuanzaizainalishangban",
            "xuan'zai'zai'na'li'shang'ban",
            Some("现在在哪里上班"),
            "xian'zai'zai'na'li'shang'ban",
        ),
        (
            3,
            "我们选在哪里见面",
            "womenxuanzainalijianmian",
            "wo'men'xuan'zai'na'li'jian'mian",
            None,
            "",
        ),
        (4, "今天心情很好", "", "", None, ""),
    ] {
        requests.push((
            Request {
                raw: raw.into(),
                reading: reading.into(),
                id: id + 5,
                text: source.into(),
            },
            expected,
            expected_reading,
        ));
    }
    for (request, expected, expected_reading) in requests {
        let started = Instant::now();
        let id = request.id;
        println!("production_client_request={request:?}");
        while !client.submit(request.clone()) {
            assert!(started.elapsed() < Duration::from_secs(1));
            std::thread::yield_now();
        }
        let response = loop {
            if let Some(response) = client.try_result() {
                break response;
            }
            assert!(
                started.elapsed() < Duration::from_millis(flower::ai::RESULT_TIMEOUT_MS + 500),
                "production client lost result"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(response.id, id);
        assert_eq!(response.corrected.as_deref(), expected);
        assert_eq!(response.reading, expected_reading);
        println!(
            "production_client_request_{id}_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    client.shutdown();
    broker.0.kill().unwrap();
    let status = broker.0.wait().unwrap();
    println!("owned_broker_exit_observed=true ({status})");
}
