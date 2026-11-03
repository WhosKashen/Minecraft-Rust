//! JNI bridge to LauncherActivity (Kotlin). Only compiled for Android.
use crate::platform::{push_picked, request_repaint, PickedFile, Platform};
use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::sys::{jint, jstring};
use jni::{JNIEnv, JavaVM};
use launcher_core::accounts::SecretStore;
use launcher_core::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct AndroidPlatform {
    vm: JavaVM,
    activity: GlobalRef,
    data_dir: PathBuf,
    secrets: Arc<AndroidSecrets>,
}

impl AndroidPlatform {
    /// # Safety
    /// `vm_ptr`/`activity_ptr` must be the valid JavaVM and Activity pointers from android-activity.
    pub unsafe fn new(vm_ptr: *mut std::ffi::c_void, activity_ptr: *mut std::ffi::c_void) -> std::result::Result<Self, String> {
        let vm = JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM).map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let obj = JObject::from_raw(activity_ptr as jni::sys::jobject);
        let activity = env.new_global_ref(&obj).map_err(|e| e.to_string())?;
        let dir: String = {
            let v = env.call_method(&obj, "dataDirPath", "()Ljava/lang/String;", &[]).map_err(|e| e.to_string())?.l().map_err(|e| e.to_string())?;
            env.get_string(&JString::from(v)).map_err(|e| e.to_string())?.into()
        };
        drop(env);
        let vm2 = JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM).map_err(|e| e.to_string())?;
        let secrets = Arc::new(AndroidSecrets { vm: vm2, activity: activity.clone() });
        Ok(Self { vm, activity, data_dir: PathBuf::from(dir), secrets })
    }

    fn call_void(&self, name: &str, sig: &str, args: &[JValue]) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            if env.call_method(&self.activity, name, sig, args).is_err() {
                let _ = env.exception_clear();
                launcher_core::logs::error(&format!("JNI call {name} failed"));
            }
        }
    }
    fn call_int(&self, name: &str, sig: &str, args: &[JValue]) -> i64 {
        let Ok(mut env) = self.vm.attach_current_thread() else { return 0 };
        match env.call_method(&self.activity, name, sig, args) {
            Ok(v) => v.j().or_else(|_| v.i().map(|i| i as i64)).unwrap_or(0),
            Err(_) => { let _ = env.exception_clear(); 0 }
        }
    }
}

impl Platform for AndroidPlatform {
    fn pick_file(&self, kind: &str) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            if let Ok(k) = env.new_string(kind) { self.call_void("pickFile", "(Ljava/lang/String;)V", &[JValue::Object(&k)]); }
        }
    }
    fn export_file(&self, src: &Path, name: &str, mime: &str) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            let (Ok(a), Ok(b), Ok(c)) = (env.new_string(src.to_string_lossy()), env.new_string(name), env.new_string(mime)) else { return };
            self.call_void("exportFile", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&a), JValue::Object(&b), JValue::Object(&c)]);
        }
    }
    fn start_game(&self, plan: &Path) -> std::result::Result<(), String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let p = env.new_string(plan.to_string_lossy()).map_err(|e| e.to_string())?;
        env.call_method(&self.activity, "startGame", "(Ljava/lang/String;)V", &[JValue::Object(&p)]).map_err(|e| { let _ = env.exception_clear(); e.to_string() })?;
        Ok(())
    }
    fn open_url(&self, url: &str) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            if let Ok(u) = env.new_string(url) { self.call_void("openUrl", "(Ljava/lang/String;)V", &[JValue::Object(&u)]); }
        }
    }
    fn total_ram_mb(&self) -> u32 { self.call_int("totalRamMb", "()J", &[]).max(0) as u32 }
    fn usable_size(&self) -> (u32, u32) {
        let packed = self.call_int("usableSizePacked", "()J", &[]);
        (((packed >> 32) & 0xffff_ffff) as u32, (packed & 0xffff_ffff) as u32)
    }
    fn show_keyboard(&self, show: bool) { self.call_void("showKeyboard", "(Z)V", &[JValue::Bool(show as u8)]); }
    fn secrets(&self) -> Arc<dyn SecretStore> { self.secrets.clone() }
    fn data_dir(&self) -> PathBuf { self.data_dir.clone() }
}

/// Secrets live in the Android Keystore-backed store implemented in SecureStore.kt.
pub struct AndroidSecrets { vm: JavaVM, activity: GlobalRef }
// JavaVM and GlobalRef are Send + Sync.
impl SecretStore for AndroidSecrets {
    fn get(&self, key: &str) -> Option<String> {
        let mut env = self.vm.attach_current_thread().ok()?;
        let k = env.new_string(key).ok()?;
        let r = env.call_method(&self.activity, "secretGet", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&k)]).ok()?.l().ok()?;
        if r.is_null() { return None; }
        let s: String = env.get_string(&JString::from(r)).ok()?.into();
        Some(s)
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut env = self.vm.attach_current_thread().map_err(|e| Error::invalid(e.to_string()))?;
        let (k, v) = (env.new_string(key).map_err(|e| Error::invalid(e.to_string()))?, env.new_string(value).map_err(|e| Error::invalid(e.to_string()))?);
        env.call_method(&self.activity, "secretSet", "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&k), JValue::Object(&v)])
            .map_err(|e| { let _ = env.exception_clear(); Error::invalid(format!("Secure storage failed: {e}")) })?;
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut env = self.vm.attach_current_thread().map_err(|e| Error::invalid(e.to_string()))?;
        let k = env.new_string(key).map_err(|e| Error::invalid(e.to_string()))?;
        let _ = env.call_method(&self.activity, "secretDelete", "(Ljava/lang/String;)V", &[JValue::Object(&k)]);
        let _ = env.exception_clear();
        Ok(())
    }
}

/// Called by LauncherActivity after the picked document was copied into the app's imports dir.
#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_LauncherActivity_nativeOnFilePicked(mut env: JNIEnv, _this: JObject, kind: JString, path: JString, name: JString) {
    let get = |env: &mut JNIEnv, s: &JString| -> String { env.get_string(s).map(|j| j.into()).unwrap_or_default() };
    let (k, p, n) = (get(&mut env, &kind), get(&mut env, &path), get(&mut env, &name));
    push_picked(PickedFile { kind: k, path: PathBuf::from(p), display_name: n });
    request_repaint();
}

#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_LauncherActivity_nativeVersion(env: JNIEnv, _this: JObject) -> jstring {
    env.new_string(env!("CARGO_PKG_VERSION")).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ------------------------------------------------------------------ game-process input (GameHostActivity)

use launcher_core::controls::{InputEvent, InputRouter, Layout, TouchEvent, TouchPhase, MouseButton};
use std::sync::Mutex;
static ROUTER: Mutex<Option<InputRouter>> = Mutex::new(None);

fn encode(ev: &[InputEvent]) -> String {
    ev.iter().map(|e| match e {
        InputEvent::KeyDown(k) => format!("kd:{k}"),
        InputEvent::KeyUp(k) => format!("ku:{k}"),
        InputEvent::MouseDown(b) => format!("md:{}", mb(*b)),
        InputEvent::MouseUp(b) => format!("mu:{}", mb(*b)),
        InputEvent::MouseMove { dx, dy } => format!("mm:{dx:.2},{dy:.2}"),
        InputEvent::Scroll(s) => format!("sc:{s}"),
    }).collect::<Vec<_>>().join("\n")
}
fn mb(b: MouseButton) -> &'static str { match b { MouseButton::Left => "L", MouseButton::Right => "R", MouseButton::Middle => "M" } }

/// Loads the layout (JSON) and usable-area aspect ratio into the gameplay router.
#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativeInit(mut env: JNIEnv, _c: JObject, layout_json: JString, aspect: jni::sys::jfloat) -> jint {
    let json: String = env.get_string(&layout_json).map(|s| s.into()).unwrap_or_default();
    let layout = Layout::from_json(&json).unwrap_or_else(|_| launcher_core::controls::default_layout());
    *ROUTER.lock().unwrap() = Some(InputRouter::new(layout, aspect));
    0
}

/// phase: 0=down 1=move 2=up 3=cancel. x/y are normalized to the usable game area. Returns newline-separated events.
#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativeTouch(env: JNIEnv, _c: JObject, pointer: jint, phase: jint, x: jni::sys::jfloat, y: jni::sys::jfloat) -> jstring {
    let phase = match phase { 0 => TouchPhase::Down, 1 => TouchPhase::Move, 2 => TouchPhase::Up, _ => TouchPhase::Cancel };
    let out = match ROUTER.lock().unwrap().as_mut() {
        Some(r) => encode(&r.handle(TouchEvent { pointer: pointer as u32, phase, x, y })),
        None => String::new(),
    };
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Must be called on pause / focus loss / stop so nothing stays pressed.
#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativeReleaseAll(env: JNIEnv, _c: JObject) -> jstring {
    let out = match ROUTER.lock().unwrap().as_mut() { Some(r) => encode(&r.release_all()), None => String::new() };
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ------------------------------------------------------------------ controller input

use launcher_core::controls::GamepadRouter;
static PAD: Mutex<Option<GamepadRouter>> = Mutex::new(None);
fn with_pad<R>(f: impl FnOnce(&mut GamepadRouter) -> R) -> R {
    let mut g = PAD.lock().unwrap();
    f(g.get_or_insert_with(GamepadRouter::new))
}

#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativePadButton(mut env: JNIEnv, _c: JObject, name: JString, pressed: jni::sys::jboolean) -> jstring {
    let n: String = env.get_string(&name).map(|s| s.into()).unwrap_or_default();
    let out = encode(&with_pad(|p| p.button(&n, pressed != 0)));
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativePadLeftStick(env: JNIEnv, _c: JObject, x: jni::sys::jfloat, y: jni::sys::jfloat) -> jstring {
    let out = encode(&with_pad(|p| p.left_stick(x, y)));
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// dt in seconds since the previous call.
#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativePadRightStick(env: JNIEnv, _c: JObject, x: jni::sys::jfloat, y: jni::sys::jfloat, dt: jni::sys::jfloat) -> jstring {
    let out = with_pad(|p| p.right_stick(x, y, dt)).map(|e| encode(&[e])).unwrap_or_default();
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_dev_rustcraft_launcher_GameInput_nativePadReleaseAll(env: JNIEnv, _c: JObject) -> jstring {
    let out = encode(&with_pad(|p| p.release_all()));
    env.new_string(out).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
