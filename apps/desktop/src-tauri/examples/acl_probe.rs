//! Explicit macOS development acceptance executable, never a product command/bridge.
//! Uses production handlers and capability manifest in two real WebViews.
use hiroute_desktop::{
    bootstrap::Resident,
    bridge::{self, DesktopState},
    session::Session,
};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::Duration,
};
use tauri::Manager;

fn main() {
    const { assert!(cfg!(debug_assertions), "development acceptance only") };
    let root = std::path::PathBuf::from(
        std::env::var_os("HIROUTE_DESKTOP_TEST_ROOT").expect("isolated root required"),
    );
    let report = root.join("acl-probe.json");
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    let resident = Resident::open(&root, &binary).unwrap();
    let session = Session::new(resident);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut stream = connection.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>HiRoute remote ACL probe</title><h1>Remote-origin permission probe</h1>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .manage(DesktopState(Arc::new(tokio::sync::Mutex::new(Some(session))), Default::default(), AtomicU64::new(0), hiroute_diagnostics::context::DiagnosticHandle::noop()))
        .invoke_handler(tauri::generate_handler![bridge::desktop_snapshot, bridge::preview_rename, bridge::preview_restore_name, bridge::observe_operation, bridge::stop_observing])
        .setup(move |app| {
            for (label, source) in [("unpermitted", tauri::WebviewUrl::App("index.html".into())), ("main", tauri::WebviewUrl::External(url.parse().unwrap()))] {
                let results = results.clone(); let report = report.clone(); let handle = app.handle().clone();
                tauri::WebviewWindowBuilder::new(app, label, source)
                    .title(format!("HiRoute ACL acceptance · {label}"))
                    .initialization_script(r#"
document.addEventListener('DOMContentLoaded', async () => {
  const commands = ['desktop_snapshot','preview_rename','preview_restore_name','observe_operation','stop_observing'];
  const results = [];
  for (const command of commands) {
    try { await window.__TAURI_INTERNALS__.invoke(command, {input:{plan_id:'plan/acl-probe',display_name:'must not apply',language:'en'}}); results.push({command, allowed:true}); }
    catch (error) { results.push({command, allowed:false, error:String(error)}); }
  }
  location.href = 'https://hiroute-acceptance.invalid/result?value=' + encodeURIComponent(JSON.stringify({origin:location.href,results}));
});
"#)
                    .on_navigation(move |url| {
                        if url.host_str() != Some("hiroute-acceptance.invalid") { return true; }
                        let value = url.query_pairs().find(|(k,_)| k == "value").unwrap().1.into_owned();
                        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
                        let mut results = results.lock().unwrap(); results.push(value);
                        if results.len() == 2 {
                            let valid = results.iter().all(|r| r["results"].as_array().is_some_and(|rows| rows.len()==5 && rows.iter().all(|row| row["allowed"] == false && row["error"].as_str().is_some_and(|e| e.contains("not allowed") || e.contains("not permitted")))));
                            std::fs::write(&report, serde_json::to_vec_pretty(&*results).unwrap()).unwrap();
                            handle.exit(if valid {0} else {1});
                        }
                        false
                    }).build()?;
            }
            let handle = app.handle().clone(); std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(30)); handle.exit(2); });
            Ok(())
        }).build(context).expect("real ACL probe application");
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            let state = handle.state::<DesktopState>().0.clone();
            tauri::async_runtime::block_on(async {
                drop(state.lock().await.take());
            });
        }
    });
}
