use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use tokio::process::Child;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(super) struct WindowsJob {
    handle: Option<OwnedHandle>,
}

impl WindowsJob {
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: null security/name pointers request an unnamed job with default
        // security. A successful raw handle is immediately owned by OwnedHandle.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a fresh owned HANDLE above.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: handle is a live job handle and the pointer/size describe the
        // initialized JOBOBJECT_EXTENDED_LIMIT_INFORMATION value for this call.
        let configured = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle().cast(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle: Some(handle),
        })
    }

    pub(super) fn assign(&self, child: &Child) -> io::Result<()> {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "child already exited"))?;
        let job = self
            .handle
            .as_ref()
            .expect("job is open until containment cleanup");
        // SAFETY: both handles are live for the duration of the call. The job
        // remains owned by self and the process remains owned by child.
        let assigned = unsafe {
            AssignProcessToJobObject(
                job.as_raw_handle().cast::<std::ffi::c_void>() as HANDLE,
                process.cast::<std::ffi::c_void>() as HANDLE,
            )
        };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn request_termination(&self) -> io::Result<()> {
        let job = self
            .handle
            .as_ref()
            .expect("job remains open until cleanup is confirmed");
        // SAFETY: job is a live Job Object handle owned by self.
        let terminated = unsafe { TerminateJobObject(job.as_raw_handle().cast(), 1) };
        if terminated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn active_processes(&self) -> io::Result<u32> {
        let job = self
            .handle
            .as_ref()
            .expect("job remains open until cleanup is confirmed");
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: job is live and accounting points to writable storage of the
        // exact type/size requested by JobObjectBasicAccountingInformation.
        let queried = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle().cast(),
                JobObjectBasicAccountingInformation,
                std::ptr::from_mut(&mut accounting).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses)
    }

    pub(super) fn close_confirmed(&mut self) {
        // Normal cleanup closes only after TerminateJobObject succeeded,
        // ActiveProcesses reached zero, and the root wait was confirmed.
        self.handle.take();
    }
}
