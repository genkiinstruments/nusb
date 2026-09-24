use std::{ffi::c_void, mem::size_of_val, ptr};

use log::warn;
use windows_sys::{
    core::BOOL,
    Win32::{
        Devices::Usb::{
            self, WinUsb_ControlTransfer, WinUsb_Free, WinUsb_GetAssociatedInterface,
            WinUsb_Initialize, WinUsb_ReadPipe, WinUsb_ResetPipe,
            WinUsb_SetCurrentAlternateSetting, WinUsb_SetPipePolicy, WinUsb_WritePipe,
            WINUSB_INTERFACE_HANDLE, WINUSB_SETUP_PACKET,
        },
        Foundation::{GetLastError, FALSE, HANDLE, WIN32_ERROR},
        System::IO::OVERLAPPED,
    },
};

use crate::transfer::Direction;

fn check(r: BOOL) -> Result<(), WIN32_ERROR> {
    if r == FALSE {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

pub fn initialize(handle: HANDLE) -> Result<WINUSB_INTERFACE_HANDLE, WIN32_ERROR> {
    let mut h = ptr::null_mut();
    check(unsafe { WinUsb_Initialize(handle, &mut h) })?;

    // Disable WinUSB's default control transfer timeout so we can do our own
    // per-request timeout handling by cancelling the transfer with a timer.

    let timeout: u32 = 0;
    let r = unsafe {
        WinUsb_SetPipePolicy(
            h,
            0x00,
            Usb::PIPE_TRANSFER_TIMEOUT,
            size_of_val(&timeout) as u32,
            &timeout as *const _ as *const c_void,
        )
    };
    if let Err(err) = check(r) {
        warn!("Failed to disable default timeout on control endpoint, error {err}");
    }

    Ok(h)
}

pub fn associated_interface(
    h: WINUSB_INTERFACE_HANDLE,
    index: u8,
) -> Result<WINUSB_INTERFACE_HANDLE, WIN32_ERROR> {
    let mut out = ptr::null_mut();
    check(unsafe { WinUsb_GetAssociatedInterface(h, index, &mut out) })?;
    Ok(out)
}

pub fn free(h: WINUSB_INTERFACE_HANDLE) {
    unsafe { WinUsb_Free(h) };
}

pub fn set_alt_setting(h: WINUSB_INTERFACE_HANDLE, alt_setting: u8) -> Result<(), WIN32_ERROR> {
    check(unsafe { WinUsb_SetCurrentAlternateSetting(h, alt_setting) })
}

pub fn enable_raw_io(h: WINUSB_INTERFACE_HANDLE, endpoint: u8) {
    let enable: u8 = 1;
    let r = unsafe {
        WinUsb_SetPipePolicy(
            h,
            endpoint,
            Usb::RAW_IO,
            size_of_val(&enable) as u32,
            &enable as *const _ as *const c_void,
        )
    };
    if let Err(err) = check(r) {
        warn!("Failed to enable RAW_IO on endpoint {endpoint:02X}: error {err:x}");
    }
}

pub unsafe fn submit(
    h: WINUSB_INTERFACE_HANDLE,
    endpoint: u8,
    buf: *mut u8,
    len: u32,
    overlapped: *mut OVERLAPPED,
) -> BOOL {
    match Direction::from_address(endpoint) {
        Direction::Out => WinUsb_WritePipe(h, endpoint, buf, len, ptr::null_mut(), overlapped),
        Direction::In => WinUsb_ReadPipe(h, endpoint, buf, len, ptr::null_mut(), overlapped),
    }
}

pub unsafe fn submit_control(
    h: WINUSB_INTERFACE_HANDLE,
    pkt: WINUSB_SETUP_PACKET,
    buf: *mut u8,
    len: u32,
    overlapped: *mut OVERLAPPED,
) -> BOOL {
    WinUsb_ControlTransfer(h, pkt, buf, len, ptr::null_mut(), overlapped)
}

pub fn reset_pipe(h: WINUSB_INTERFACE_HANDLE, endpoint: u8) -> Result<(), WIN32_ERROR> {
    check(unsafe { WinUsb_ResetPipe(h, endpoint) })
}
