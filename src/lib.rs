#![crate_id   = "github.com/hannobraun/inotify-rs#inotify:0.1"]
#![crate_type = "lib"]

extern crate libc;

pub use wrapper::INotify;

pub mod ffi;
pub mod wrapper;
