//! Rustcraft launcher UI (egui) and Android entry point.
pub mod app;
mod pages;
pub mod platform;
#[cfg(target_os = "android")]
pub mod android;

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use std::sync::Arc;
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("Rustcraft"));
    let platform = match unsafe { android::AndroidPlatform::new(app.vm_as_ptr() as *mut _, app.activity_as_ptr() as *mut _) } {
        Ok(p) => Arc::new(p) as Arc<dyn platform::Platform>,
        Err(e) => { log::error!("platform init failed: {e}"); return; }
    };
    let options = eframe::NativeOptions { android_app: Some(app), renderer: eframe::Renderer::Glow, ..Default::default() };
    let res = eframe::run_native("Rustcraft", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, platform.clone())))));
    if let Err(e) = res { log::error!("eframe exited with error: {e}"); }
}
