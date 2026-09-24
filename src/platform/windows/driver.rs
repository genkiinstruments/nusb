use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use windows_sys::{
    core::BOOL,
    Win32::{
        Devices::Usb::WINUSB_INTERFACE_HANDLE,
        Foundation::{HANDLE, WIN32_ERROR},
        System::IO::OVERLAPPED,
    },
};

use crate::MaybeFuture;

use super::{libusb0, winusb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Driver {
    WinUsb,
    Libusb0,
}

impl Driver {
    pub(crate) fn from_name(name: &str) -> Option<Driver> {
        if name.eq_ignore_ascii_case("winusb") {
            Some(Driver::WinUsb)
        } else if name.eq_ignore_ascii_case("libusb0") {
            Some(Driver::Libusb0)
        } else {
            None
        }
    }

    pub(crate) fn open(self, handle: HANDLE) -> Result<DriverHandle, WIN32_ERROR> {
        match self {
            Driver::WinUsb => winusb::initialize(handle).map(DriverHandle::WinUsb),
            Driver::Libusb0 => Ok(DriverHandle::Libusb0(Arc::default())),
        }
    }
}

#[derive(Clone)]
pub(crate) enum DriverHandle {
    WinUsb(WINUSB_INTERFACE_HANDLE),
    Libusb0(Arc<libusb0::ControlPipe>),
}

impl DriverHandle {
    pub(crate) fn set_alt_setting(
        &self,
        handle: HANDLE,
        interface: u8,
        alt_setting: u8,
    ) -> Result<(), WIN32_ERROR> {
        match self {
            DriverHandle::WinUsb(h) => winusb::set_alt_setting(*h, alt_setting),
            DriverHandle::Libusb0(_) => libusb0::set_alt_setting(handle, interface, alt_setting),
        }
    }

    pub(crate) unsafe fn submit(
        &self,
        handle: HANDLE,
        endpoint: u8,
        buf: *mut u8,
        len: u32,
        overlapped: *mut OVERLAPPED,
    ) -> BOOL {
        match self {
            DriverHandle::WinUsb(h) => winusb::submit(*h, endpoint, buf, len, overlapped),
            DriverHandle::Libusb0(_) => libusb0::submit(handle, endpoint, buf, len, overlapped),
        }
    }

    pub(crate) fn clear_halt(&self, handle: HANDLE, endpoint: u8) -> Result<(), WIN32_ERROR> {
        match self {
            DriverHandle::WinUsb(h) => winusb::reset_pipe(*h, endpoint),
            DriverHandle::Libusb0(_) => libusb0::reset_endpoint(handle, endpoint),
        }
    }
}

/// A control transfer on either driver
pub(crate) enum ControlTransfer<W, L> {
    WinUsb(W),
    Libusb0(L),
}

impl<W: IntoFuture, L: IntoFuture<Output = W::Output>> IntoFuture for ControlTransfer<W, L> {
    type Output = W::Output;
    type IntoFuture = ControlTransferFuture<W::IntoFuture, L::IntoFuture>;

    fn into_future(self) -> Self::IntoFuture {
        match self {
            ControlTransfer::WinUsb(w) => ControlTransferFuture::WinUsb(w.into_future()),
            ControlTransfer::Libusb0(l) => ControlTransferFuture::Libusb0(l.into_future()),
        }
    }
}

impl<W: MaybeFuture, L: MaybeFuture<Output = W::Output>> MaybeFuture for ControlTransfer<W, L> {
    fn wait(self) -> Self::Output {
        match self {
            ControlTransfer::WinUsb(w) => w.wait(),
            ControlTransfer::Libusb0(l) => l.wait(),
        }
    }
}

pub(crate) enum ControlTransferFuture<W, L> {
    WinUsb(W),
    Libusb0(L),
}

impl<W: Future, L: Future<Output = W::Output>> Future for ControlTransferFuture<W, L> {
    type Output = W::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: structural pin projection: the active variant is never moved.
        unsafe {
            match self.get_unchecked_mut() {
                ControlTransferFuture::WinUsb(w) => Pin::new_unchecked(w).poll(cx),
                ControlTransferFuture::Libusb0(l) => Pin::new_unchecked(l).poll(cx),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maybe_future::{blocking::Blocking, Ready};

    fn transfer(driver: Driver) -> impl MaybeFuture<Output = Driver> {
        match driver {
            Driver::WinUsb => ControlTransfer::WinUsb(Ready(Driver::WinUsb)),
            Driver::Libusb0 => ControlTransfer::Libusb0(Blocking::new(|| Driver::Libusb0)),
        }
    }

    #[test]
    fn driver_from_name() {
        assert_eq!(Driver::from_name("WinUSB"), Some(Driver::WinUsb));
        assert_eq!(Driver::from_name("libusb0"), Some(Driver::Libusb0));
        assert_eq!(Driver::from_name("usbccgp"), None);
    }

    #[test]
    fn control_transfer_wait() {
        assert_eq!(transfer(Driver::WinUsb).wait(), Driver::WinUsb);
        assert_eq!(transfer(Driver::Libusb0).wait(), Driver::Libusb0);
    }

    #[cfg(any(feature = "smol", feature = "tokio"))]
    #[tokio::test]
    async fn control_transfer_await() {
        assert_eq!(transfer(Driver::WinUsb).await, Driver::WinUsb);
        assert_eq!(transfer(Driver::Libusb0).await, Driver::Libusb0);
    }
}
