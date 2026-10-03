use std::{
    fs::File,
    io::{Read, Write},
    os::fd::FromRawFd,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[repr(C)]
struct WindowSize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

#[cfg_attr(target_os = "linux", link(name = "util"))]
extern "C" {
    fn openpty(
        master: *mut i32,
        slave: *mut i32,
        name: *mut std::ffi::c_char,
        termios: *const std::ffi::c_void,
        size: *const WindowSize,
    ) -> i32;
}

// Ratatui emits cursor moves instead of spaces between unchanged blank cells.
fn plain_text(output: &str) -> String {
    let mut chars = output.chars();
    let mut text = String::new();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.next() == Some('[') {
                for character in chars.by_ref() {
                    if ('@'..='~').contains(&character) {
                        break;
                    }
                }
            }
        } else if !character.is_whitespace() && !character.is_control() {
            text.push(character);
        }
    }
    text
}

pub struct Session {
    child: Child,
    input: File,
    chunks: Receiver<Vec<u8>>,
    output: String,
}

impl Session {
    pub fn start(mode: &str) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let size = WindowSize {
            rows: 20,
            cols: 100,
            xpixel: 0,
            ypixel: 0,
        };
        // openpty initializes both descriptors; each is immediately owned by one File.
        let result = unsafe {
            openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        assert_eq!(result, 0, "openpty: {}", std::io::Error::last_os_error());
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_picker_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("DEFBROW_TEST_PICKER", mode)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap();
        let mut reader = master.try_clone().unwrap();
        let (sender, chunks) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: master,
            chunks,
            output: String::new(),
        }
    }

    pub fn wait_for(&mut self, marker: &str) {
        self.wait_for_all(&[marker]);
    }

    pub fn wait_for_all(&mut self, markers: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let markers: Vec<_> = markers.iter().map(|marker| plain_text(marker)).collect();
        while !markers
            .iter()
            .all(|marker| plain_text(&self.output).contains(marker))
        {
            let chunk = self
                .chunks
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| panic!("Timed out waiting for {markers:?}: {:?}", self.output));
            self.output.push_str(&String::from_utf8_lossy(&chunk));
        }
        self.output.clear();
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).unwrap();
        self.input.flush().unwrap();
    }

    pub fn finish(&mut self) {
        self.wait_for("MOCK-SESSION-CANCELLED");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "fixture failed: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "fixture did not exit");
            thread::yield_now();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
