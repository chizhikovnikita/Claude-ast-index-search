use std::fs::File;
use std::io;
use std::mem::{size_of, size_of_val, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use anyhow::{Context, Result};
use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, CREATE_NEW_PROCESS_GROUP,
    DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

struct Attributes(LPPROC_THREAD_ATTRIBUTE_LIST);

impl Drop for Attributes {
    fn drop(&mut self) {
        // The allocation backing the initialized list outlives this guard.
        unsafe { DeleteProcThreadAttributeList(self.0) };
    }
}

fn inheritable_handle(file: &File) -> io::Result<OwnedHandle> {
    let mut handle = null_mut();
    // Duplicate only the worker's actual streams, keeping their owners alive until spawn.
    let success = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            file.as_raw_handle(),
            GetCurrentProcess(),
            &mut handle,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    // DuplicateHandle returned a new owned handle on success.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

pub(super) fn spawn(
    root: &Path,
    executable: &Path,
    log: &File,
    debounce_ms: u64,
    verbose: bool,
    token: u64,
) -> Result<()> {
    let stdin = inheritable_handle(&File::open("NUL")?)?;
    let output = inheritable_handle(log)?;
    let mut handles = [stdin.as_raw_handle(), output.as_raw_handle()];
    let mut bytes = 0;
    // The first call obtains the required size for one process attribute.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
    anyhow::ensure!(bytes > 0, "failed to size worker process attributes");
    let mut storage = vec![0u128; bytes.div_ceil(size_of::<u128>())];
    let list = storage.as_mut_ptr().cast();
    let initialized = unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) };
    if initialized == 0 {
        return Err(io::Error::last_os_error()).context("failed to initialize worker attributes");
    }
    let attributes = Attributes(list);
    // Rust's default Windows spawn inherits unrelated pipe handles. Restricting
    // inheritance prevents a detached worker from holding the caller's pipes open.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attributes.0,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_mut_ptr().cast(),
            size_of_val(&handles),
            null_mut(),
            null(),
        )
    };
    if updated == 0 {
        return Err(io::Error::last_os_error()).context("failed to restrict worker handles");
    }
    let application: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let directory: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut arguments = vec![u16::from(b'"')];
    arguments.extend(executable.as_os_str().encode_wide());
    arguments.extend(
        format!(
            "\" update --coordinator-worker --coordinator-claim {token} --debounce-ms {debounce_ms}{}",
            if verbose { " --verbose" } else { "" },
        )
        .encode_utf16(),
    );
    arguments.push(0);
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin.as_raw_handle();
    startup.StartupInfo.hStdOutput = output.as_raw_handle();
    startup.StartupInfo.hStdError = output.as_raw_handle();
    startup.lpAttributeList = attributes.0;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    // All pointers refer to live buffers; only the two allow-listed handles inherit.
    let success = unsafe {
        CreateProcessW(
            application.as_ptr(),
            arguments.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | EXTENDED_STARTUPINFO_PRESENT,
            null(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error()).context("failed to spawn update worker");
    }
    // Closing our process/thread handles does not wait for the detached worker.
    unsafe {
        drop(OwnedHandle::from_raw_handle(process.hThread));
        drop(OwnedHandle::from_raw_handle(process.hProcess));
    }
    Ok(())
}
