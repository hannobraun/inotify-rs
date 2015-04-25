#![crate_name = "inotify"]
#![crate_type = "lib"]
#![warn(missing_docs)]

//! Binding and wrapper for inotify.
//!
//! [Inotify][wiki] is a linux kernel mechanism for monitoring
//! changes to filesystems' contents.
//!
//! > The inotify API provides a mechanism for monitoring filesystem
//! > events. Inotify can be used to monitor individual files, or to
//! > monitor directories. When a directory is monitored, inotify will
//! > return events for the directory itself, and for files inside the
//! > directory.
//!
//! See the [man page][inotify7] for usage information
//! of the C version, which this package follows closely.
//!
//! [wiki]: https://en.wikipedia.org/wiki/Inotify
//! [inotify7]: http://man7.org/linux/man-pages/man7/inotify.7.html

extern crate libc;

pub use wrapper::INotify;

pub mod ffi;
pub mod wrapper;
