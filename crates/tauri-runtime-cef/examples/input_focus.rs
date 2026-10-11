//! Native fixture for scripts/check-input-focus.mjs.

const PAGE: &str = r#"<!doctype html>
<input id="input" style="position:fixed;left:40px;top:60px;width:400px;height:40px">
<textarea id="textarea" style="position:fixed;left:40px;top:160px;width:400px;height:40px"></textarea>
<div id="editable" contenteditable="true" style="position:fixed;left:40px;top:260px;width:400px;height:40px"></div>
"#;

fn main() {
  tauri_runtime_cef::configure(tauri_runtime_cef::CefConfig {
    identifier: "sable-input-focus-probe".into(),
    custom_schemes: vec![
      "tauri".into(),
      "ipc".into(),
      "asset".into(),
      "focus-probe".into(),
    ],
    command_line_args: vec![
      ("no-sandbox".into(), None),
      ("disable-gpu".into(), None),
      (
        "remote-debugging-port".into(),
        Some(std::env::var("CEF_FOCUS_PORT").expect("CEF_FOCUS_PORT is required")),
      ),
    ],
    ..Default::default()
  });
  if std::env::args().any(|arg| arg.starts_with("--type=")) {
    tauri_runtime_cef::run_cef_helper_process();
    return;
  }

  type Rt = tauri_runtime_cef::CefRuntime<tauri::EventLoopMessage>;
  tauri::Builder::<Rt>::new()
    .register_uri_scheme_protocol("focus-probe", |_context, _request| {
      tauri::http::Response::builder()
        .header("content-type", "text/html")
        .body(PAGE.as_bytes().to_vec())
        .unwrap()
    })
    .setup(|app| {
      for label in ["a", "b"] {
        tauri::WebviewWindowBuilder::new(
          app,
          label,
          tauri::WebviewUrl::External(format!("focus-probe://localhost/{label}").parse().unwrap()),
        )
        .title(format!("Sable focus probe {label}"))
        .inner_size(600., 500.)
        .position(if label == "a" { 50. } else { 750. }, 50.)
        .build()?;
      }
      println!("FOCUS-PROBE-READY");
      Ok(())
    })
    .run(tauri::test::mock_context(tauri::test::noop_assets()))
    .expect("input focus probe");
}
