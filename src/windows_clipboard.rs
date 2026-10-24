//! Eagerly rendered Win32 Unicode clipboard in the current interactive desktop.
use super::*;
use std::{ffi::c_void, ptr::null_mut};

type Handle = *mut c_void;
#[repr(C)]
struct UserFlags {
    inherit: i32,
    reserved: i32,
    flags: u32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetProcessWindowStation() -> Handle;
    fn GetUserObjectInformationW(
        object: Handle,
        index: i32,
        data: *mut c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
    fn GetThreadDesktop(thread: u32) -> Handle;
    fn OpenInputDesktop(flags: u32, inherit: i32, access: u32) -> Handle;
    fn CloseDesktop(desktop: Handle) -> i32;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: Handle,
    ) -> Handle;
    fn DestroyWindow(window: Handle) -> i32;
    fn OpenClipboard(window: Handle) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> Handle;
    fn SetClipboardData(format: u32, data: Handle) -> Handle;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    fn GlobalFree(memory: Handle) -> Handle;
    fn GlobalLock(memory: Handle) -> Handle;
    fn GlobalUnlock(memory: Handle) -> i32;
    fn GlobalSize(memory: Handle) -> usize;
}

const UNICODE_TEXT: u32 = 13;

fn desktop_name(handle: Handle) -> Option<Vec<u16>> {
    let mut name = [0u16; 256];
    let mut needed = 0;
    // UOI_NAME; the bounded name contains no clipboard/business data.
    let result =
        unsafe { GetUserObjectInformationW(handle, 2, name.as_mut_ptr().cast(), 512, &mut needed) };
    if result == 0 || needed as usize > std::mem::size_of_val(&name) {
        return None;
    }
    Some(name.into_iter().take_while(|unit| *unit != 0).collect())
}
fn interactive() -> Result<(), ClipboardError> {
    let mut flags = UserFlags {
        inherit: 0,
        reserved: 0,
        flags: 0,
    };
    let mut needed = 0;
    unsafe {
        let station = GetProcessWindowStation();
        if station.is_null()
            || GetUserObjectInformationW(
                station,
                1,
                (&mut flags as *mut UserFlags).cast(),
                std::mem::size_of::<UserFlags>() as u32,
                &mut needed,
            ) == 0
            || flags.flags & 1 == 0
        {
            return Err(ClipboardError::Unavailable);
        }
        let input = OpenInputDesktop(0, 0, 1); // DESKTOP_READOBJECTS
        if input.is_null() {
            return Err(ClipboardError::Unavailable);
        }
        let current = GetThreadDesktop(GetCurrentThreadId());
        let input_name = desktop_name(input);
        let current_name = desktop_name(current);
        CloseDesktop(input);
        if input_name.is_none() || input_name != current_name {
            return Err(ClipboardError::Unavailable);
        }
    }
    Ok(())
}

struct ClipboardGuard {
    window: Handle,
}
impl ClipboardGuard {
    fn open() -> Result<Self, ClipboardError> {
        interactive()?;
        let class: Vec<_> = "STATIC\0".encode_utf16().collect();
        // A private invisible window supplies a real owner. OpenClipboard(NULL)
        // followed by EmptyClipboard would leave no owner for SetClipboardData.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if window.is_null() {
            return Err(ClipboardError::Unavailable);
        }
        for _ in 0..10 {
            if unsafe { OpenClipboard(window) } != 0 {
                return Ok(Self { window });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        unsafe {
            DestroyWindow(window);
        }
        Err(ClipboardError::Unavailable)
    }
}
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
            DestroyWindow(self.window);
        }
    }
}
fn read() -> Result<Option<String>, ClipboardError> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if IsClipboardFormatAvailable(UNICODE_TEXT) == 0 {
            return Ok(None);
        }
        let memory = GetClipboardData(UNICODE_TEXT);
        if memory.is_null() {
            return Err(ClipboardError::Read);
        }
        let bytes = GlobalSize(memory);
        if bytes > MAX_TEXT_BYTES * 2 + 2 {
            return Err(ClipboardError::TooLarge);
        }
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            return Err(ClipboardError::Read);
        }
        let units = std::slice::from_raw_parts(pointer.cast::<u16>(), bytes / 2);
        let result = units
            .iter()
            .position(|unit| *unit == 0)
            .ok_or(ClipboardError::Read)
            .and_then(|length| {
                String::from_utf16(&units[..length]).map_err(|_| ClipboardError::Read)
            });
        GlobalUnlock(memory);
        let text = result?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        Ok(Some(text))
    }
}
fn write(text: &str) -> Result<(), ClipboardError> {
    if text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Err(ClipboardError::Write);
    }
    let units: Vec<_> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _guard = ClipboardGuard::open()?;
    unsafe {
        let memory = GlobalAlloc(2, units.len() * 2); // GMEM_MOVEABLE
        if memory.is_null() {
            return Err(ClipboardError::Write);
        }
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            GlobalFree(memory);
            return Err(ClipboardError::Write);
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast::<u16>(), units.len());
        GlobalUnlock(memory);
        if EmptyClipboard() == 0 || SetClipboardData(UNICODE_TEXT, memory).is_null() {
            GlobalFree(memory);
            return Err(ClipboardError::Write);
        }
        // The system owns memory after successful SetClipboardData.
    }
    Ok(())
}

pub struct SystemClipboard;
impl SystemClipboard {
    pub fn detect() -> Result<Self, ClipboardError> {
        interactive()?;
        Ok(Self)
    }
    pub fn capability(&self) -> ClipboardCapability {
        ClipboardCapability::available("windows/interactive-unicode")
    }
    pub async fn read(&self) -> Result<Option<String>, ClipboardError> {
        // Join the OS call; cancellation must not abandon a platform operation.
        tokio::task::spawn_blocking(read)
            .await
            .map_err(|_| ClipboardError::Read)?
    }
    pub async fn write(&self, text: &str) -> Result<(), ClipboardError> {
        let text = text.to_owned();
        tokio::task::spawn_blocking(move || write(&text))
            .await
            .map_err(|_| ClipboardError::Write)?
    }
    pub async fn probe(&self) -> Result<(), ClipboardError> {
        tokio::task::spawn_blocking(|| ClipboardGuard::open().map(drop))
            .await
            .map_err(|_| ClipboardError::Unavailable)?
    }
    pub async fn close(&self) {}
}
