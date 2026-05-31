use crate::{
    app::AppResult,
    logging::Logger,
    platform::{find_ffmpeg_binary, rename_with_context},
};
use std::{
    ffi::{c_char, c_void},
    io,
    path::{Path, PathBuf},
    process::Command,
    ptr, thread,
};

type CFAllocatorRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFStringEncoding = u32;
type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;
type Boolean = u8;

const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;
const SCREEN_LOCKED_KEY: &[u8] = b"CGSSessionScreenIsLocked\0";

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
    fn CFDictionaryGetValue(the_dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const c_char,
        encoding: CFStringEncoding,
    ) -> CFStringRef;
}

pub(crate) fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    rename_with_context(source, destination, "替换文件失败")?;
    Ok(())
}

pub(crate) fn find_ffmpeg() -> AppResult<PathBuf> {
    find_ffmpeg_binary("ffmpeg", &["../Resources/ffmpeg"])
}

pub(crate) fn notify(title: &str, message: &str, logger: Logger) {
    let title = title.to_string();
    let message = message.to_string();
    thread::spawn(move || {
        let script = format!(
            "display notification \"{}\" with title \"Screen Recorder\" subtitle \"{}\"",
            escape_applescript_string(&message),
            escape_applescript_string(&title)
        );
        let result = Command::new("osascript").args(["-e", &script]).status();

        if let Err(error) = result {
            logger.error(format!("发送通知失败: {error}"));
        }
    });
}

pub(crate) fn open_path(path: &Path) -> AppResult<()> {
    let status = Command::new("open").arg(path).status()?;
    if !status.success() {
        return Err(
            std::io::Error::other(format!("打开路径失败 {}: {status}", path.display())).into(),
        );
    }
    Ok(())
}

pub(crate) fn request_screen_capture_permission(logger: &Logger) {
    if has_screen_capture_access() {
        logger.info("macOS 屏幕录制权限已授权");
        return;
    }

    logger.info("请求 macOS 屏幕录制权限");
    if request_screen_capture_access() {
        logger.info("macOS 屏幕录制权限已授权");
    } else {
        logger.warn("macOS 屏幕录制权限未授权，用户可能需要在系统设置中授权并重启应用");
    }
}

pub(crate) fn screen_locked() -> AppResult<bool> {
    let session = unsafe { CGSessionCopyCurrentDictionary() };
    if session.is_null() {
        return Err(io::Error::other("读取 macOS 会话状态失败").into());
    }

    let key = unsafe {
        CFStringCreateWithCString(
            ptr::null(),
            SCREEN_LOCKED_KEY.as_ptr().cast::<c_char>(),
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if key.is_null() {
        unsafe {
            CFRelease(session);
        }
        return Err(io::Error::other("创建 macOS 锁屏状态键失败").into());
    }

    let value = unsafe { CFDictionaryGetValue(session, key.cast::<c_void>()) };
    let locked = !value.is_null() && unsafe { CFBooleanGetValue(value.cast::<c_void>()) != 0 };

    unsafe {
        CFRelease(key);
        CFRelease(session);
    }

    Ok(locked)
}

fn has_screen_capture_access() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

fn request_screen_capture_access() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

fn escape_applescript_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
