#![crate_name = "inotify"]
#![crate_type = "lib"]

extern crate libc;

pub use wrapper::INotify;

pub mod ffi;
pub mod wrapper;
