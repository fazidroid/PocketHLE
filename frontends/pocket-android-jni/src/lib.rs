//! Thin JNI bridge that lets the Android skeleton reach the PocketHLE
//! core. For now it exports a single `banner()` method used to prove
//! the .so links and runs.

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

#[no_mangle]
pub extern "system" fn Java_com_pockethle_app_MainActivity_banner<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    init_logger();
    let banner = format!(
        "PocketHLE v{} — Android skeleton.\n\
         Native lib loaded ok. Stub frontend; UI is not wired yet.",
        env!("CARGO_PKG_VERSION"),
    );
    let jstr: JString<'local> = env
        .new_string(banner)
        .expect("could not allocate banner JString");
    jstr.into_raw()
}

fn init_logger() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("PocketHLE"),
        );
    });
}
