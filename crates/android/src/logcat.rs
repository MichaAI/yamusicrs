//! tracing → logcat (тег `yamusicrs`): `adb logcat -s yamusicrs`.

use std::ffi::CString;
use std::io::Write;

use android_log_sys::{LogPriority, __android_log_write};
use tracing_subscriber::fmt::MakeWriter;

pub struct Logcat;

/// Копия логов в файл (`…/Android/data/<pkg>/files/logs/yamusicrs.log`): на vivo logcat
/// временами перестаёт принимать строки от приложения, а файл adb читает всегда.
static FILE: std::sync::OnceLock<parking_lot::Mutex<std::fs::File>> = std::sync::OnceLock::new();

/// Открыть файл лога (старый, если больше 2 МБ, начинается заново).
pub fn open_file(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("yamusicrs.log");
    let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024);
    let f = std::fs::OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(&path);
    if let Ok(f) = f {
        let _ = FILE.set(parking_lot::Mutex::new(f));
    }
}

/// Строка в файл лога с временем (для Kotlin — через `Core::log`).
pub fn to_file(text: &str) {
    if let Some(f) = FILE.get() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let ms = t.as_millis() % 86_400_000;
        let _ = writeln!(
            f.lock(),
            "{:02}:{:02}:{:02}.{:03} {text}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        );
    }
}

pub struct Line(Vec<u8>);

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        to_file(text);
        let prio = if text.contains(" ERROR ") {
            LogPriority::ERROR
        } else if text.contains(" WARN ") {
            LogPriority::WARN
        } else {
            LogPriority::INFO
        };
        let (Ok(tag), Ok(msg)) = (CString::new("yamusicrs"), CString::new(text.replace('\0', ""))) else {
            return;
        };
        // SAFETY: обе строки — валидные C-строки, живут до конца вызова.
        unsafe {
            __android_log_write(prio as i32, tag.as_ptr(), msg.as_ptr());
        }
    }
}

impl<'a> MakeWriter<'a> for Logcat {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line(Vec::new())
    }
}
