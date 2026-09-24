#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_usbfs;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub use linux_usbfs::*;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos_iokit;

#[cfg(target_os = "macos")]
pub use macos_iokit::*;

#[cfg(target_arch = "wasm32")]
mod webusb;

#[cfg(target_arch = "wasm32")]
pub use webusb::*;
