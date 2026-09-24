// IOCTL interface of libusb0.sys, from libusb-win32's libusb/src/driver/driver_api.h

use std::{ffi::c_void, mem, ptr, sync::Mutex, time::Duration};

use log::debug;

use windows_sys::{
    core::BOOL,
    Win32::{
        Devices::Usb::WINUSB_SETUP_PACKET,
        Foundation::{
            CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_SEM_TIMEOUT, FALSE, HANDLE,
            STATUS_TIMEOUT, TRUE, WIN32_ERROR,
        },
        System::{
            Threading::CreateEventW,
            IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED},
        },
    },
};

use crate::transfer::Direction;

const fn ctl_code(function: u32, method: u32) -> u32 {
    const FILE_DEVICE_UNKNOWN: u32 = 0x22;
    (FILE_DEVICE_UNKNOWN << 16) | (function << 2) | method
}

const METHOD_BUFFERED: u32 = 0;
const METHOD_IN_DIRECT: u32 = 1;
const METHOD_OUT_DIRECT: u32 = 2;

const IOCTL_SET_INTERFACE: u32 = ctl_code(0x803, METHOD_BUFFERED);
const IOCTL_INTERRUPT_OR_BULK_WRITE: u32 = ctl_code(0x80A, METHOD_IN_DIRECT);
const IOCTL_INTERRUPT_OR_BULK_READ: u32 = ctl_code(0x80B, METHOD_OUT_DIRECT);
const IOCTL_RESET_ENDPOINT: u32 = ctl_code(0x80E, METHOD_BUFFERED);
const IOCTL_CLAIM_INTERFACE: u32 = ctl_code(0x815, METHOD_BUFFERED);
const IOCTL_RELEASE_INTERFACE: u32 = ctl_code(0x816, METHOD_BUFFERED);
const IOCTL_SET_PIPE_POLICY: u32 = ctl_code(0x906, METHOD_BUFFERED);
const IOCTL_CONTROL_WRITE: u32 = ctl_code(0x90A, METHOD_IN_DIRECT);
const IOCTL_CONTROL_READ: u32 = ctl_code(0x90B, METHOD_OUT_DIRECT);

const DEFAULT_TIMEOUT_MS: u32 = 5000;

const PIPE_TRANSFER_TIMEOUT: u32 = 0x03;

/// `libusb_request`
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Request {
    timeout: u32,
    args: [u32; 5],
}

const _: () = assert!(mem::size_of::<Request>() == 24);

impl Request {
    fn interface(interface: u8) -> Request {
        Request {
            args: [interface.into(), 0, 0, 0, 0],
            ..Default::default()
        }
    }

    fn alt_setting(interface: u8, alt_setting: u8) -> Request {
        Request {
            timeout: DEFAULT_TIMEOUT_MS,
            args: [interface.into(), alt_setting.into(), 0, 0, 0],
        }
    }

    fn endpoint(endpoint: u8) -> Request {
        Request {
            timeout: DEFAULT_TIMEOUT_MS,
            args: [endpoint.into(), 0, 0, 0, 0],
        }
    }

    fn control(pkt: WINUSB_SETUP_PACKET) -> Request {
        let [value_lo, value_hi] = { pkt.Value }.to_le_bytes();
        let mut args = [0; 5];
        args[0] = u32::from_le_bytes([pkt.RequestType, pkt.Request, value_lo, value_hi]);
        args[1] = u32::from(pkt.Index) | u32::from(pkt.Length) << 16;
        Request {
            args,
            ..Default::default()
        }
    }
}

#[repr(C)]
struct PipePolicy {
    request: Request,
    value: u32,
}

pub fn claim_interface(handle: HANDLE, interface: u8) -> Result<(), WIN32_ERROR> {
    ioctl(
        handle,
        IOCTL_CLAIM_INTERFACE,
        &Request::interface(interface),
    )
}

pub fn release_interface(handle: HANDLE, interface: u8) -> Result<(), WIN32_ERROR> {
    ioctl(
        handle,
        IOCTL_RELEASE_INTERFACE,
        &Request::interface(interface),
    )
}

pub fn set_alt_setting(handle: HANDLE, interface: u8, alt_setting: u8) -> Result<(), WIN32_ERROR> {
    ioctl(
        handle,
        IOCTL_SET_INTERFACE,
        &Request::alt_setting(interface, alt_setting),
    )
}

pub fn reset_endpoint(handle: HANDLE, endpoint: u8) -> Result<(), WIN32_ERROR> {
    ioctl(handle, IOCTL_RESET_ENDPOINT, &Request::endpoint(endpoint))
}

pub unsafe fn submit(
    handle: HANDLE,
    endpoint: u8,
    buf: *mut u8,
    len: u32,
    overlapped: *mut OVERLAPPED,
) -> BOOL {
    let code = match Direction::from_address(endpoint) {
        Direction::Out => IOCTL_INTERRUPT_OR_BULK_WRITE,
        Direction::In => IOCTL_INTERRUPT_OR_BULK_READ,
    };
    let req = Request::endpoint(endpoint);
    // The data goes in the output buffer in both directions
    DeviceIoControl(
        handle,
        code,
        &req as *const Request as *const c_void,
        mem::size_of_val(&req) as u32,
        buf as *mut c_void,
        len,
        ptr::null_mut(),
        overlapped,
    )
}

/// The default pipe, whose transfer timeout the driver keeps per device
#[derive(Default)]
pub struct ControlPipe(Mutex<()>);

impl ControlPipe {
    /// Blocks: the driver waits for control transfers itself.
    pub fn transfer(
        &self,
        handle: HANDLE,
        pkt: WINUSB_SETUP_PACKET,
        buf: &mut [u8],
        timeout: Duration,
    ) -> Result<u32, WIN32_ERROR> {
        debug!(
            "libusb0 control request {:02x} type {:02x} for {} bytes with timeout {timeout:?}",
            { pkt.Request },
            { pkt.RequestType },
            { pkt.Length }
        );

        let (pipe_id, code) = match Direction::from_address(pkt.RequestType) {
            Direction::In => (0x80, IOCTL_CONTROL_READ),
            Direction::Out => (0x00, IOCTL_CONTROL_WRITE),
        };

        let ms = u32::try_from(timeout.as_millis())
            .unwrap_or(u32::MAX)
            .max(1);
        let policy = PipePolicy {
            request: Request {
                args: [0, pipe_id, PIPE_TRANSFER_TIMEOUT, 0, 0],
                ..Default::default()
            },
            value: ms,
        };

        let _lock = self.0.lock().unwrap();
        ioctl(handle, IOCTL_SET_PIPE_POLICY, &policy)?;

        let req = Request::control(pkt);
        // SAFETY: the request completes before this returns, while `buf` is borrowed
        unsafe { ioctl_data(handle, code, &req, buf.as_mut_ptr(), buf.len() as u32) }
    }
}

fn ioctl<T>(handle: HANDLE, code: u32, input: &T) -> Result<(), WIN32_ERROR> {
    // SAFETY: there is no data buffer
    unsafe { ioctl_data(handle, code, input, ptr::null_mut(), 0) }.map(drop)
}

unsafe fn ioctl_data<T>(
    handle: HANDLE,
    code: u32,
    input: &T,
    data: *mut u8,
    len: u32,
) -> Result<u32, WIN32_ERROR> {
    let event = CreateEventW(ptr::null(), TRUE, FALSE, ptr::null());
    if event.is_null() {
        return Err(GetLastError());
    }

    let mut overlapped: OVERLAPPED = mem::zeroed();
    // Low bit set keeps the completion off the threadpool IO's port
    overlapped.hEvent = (event as usize | 1) as HANDLE;

    let submitted = DeviceIoControl(
        handle,
        code,
        input as *const T as *const c_void,
        mem::size_of::<T>() as u32,
        data as *mut c_void,
        len,
        ptr::null_mut(),
        &mut overlapped,
    ) != FALSE
        || GetLastError() == ERROR_IO_PENDING;

    let mut transferred = 0;
    let r =
        if submitted && GetOverlappedResult(handle, &overlapped, &mut transferred, TRUE) != FALSE {
            // A timed out request completes with STATUS_TIMEOUT, a success code
            if overlapped.Internal == STATUS_TIMEOUT as usize {
                Err(ERROR_SEM_TIMEOUT)
            } else {
                Ok(transferred)
            }
        } else {
            Err(GetLastError())
        };

    CloseHandle(event);
    r
}

#[test]
fn test_control_request_layout() {
    let r = Request::control(WINUSB_SETUP_PACKET {
        RequestType: 0x40,
        Request: 0x0c,
        Value: 0x1234,
        Index: 0x0471,
        Length: 0x1000,
    });
    // SAFETY: `Request` is `repr(C)`, 24 bytes of `u32`s
    let bytes: [u8; 24] = unsafe { mem::transmute(r) };
    assert_eq!(
        &bytes[4..12],
        &[0x40, 0x0c, 0x34, 0x12, 0x71, 0x04, 0x00, 0x10]
    );
}
