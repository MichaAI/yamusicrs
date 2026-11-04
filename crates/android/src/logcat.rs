//! tracing → logcat (тег `yamusicrs`): `adb logcat -s yamusicrs`.

use std::ffi::CString;
use std::io::Write;

use android_log_sys::{LogPriority, __android_log_write};
use tracing_subscriber::fmt::MakeWriter;

pub struct Logcat;

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
