#![crate_name = "inotify"]
#![crate_type = "lib"]
#![deny(missing_docs)]
#![deny(warnings)]

//! Idiomatic inotify wrapper for the Rust programming language
//!
//! # About
//!
//! [inotify-rs] is an idiomatic wrapper around the Linux kernel's [inotify] API
//! for the Rust programming language. It can be used for monitoring changes to
//! files or directories.
//!
//! The [`Inotify`] struct is the main entry point into the API.
//!
//! # Example
//!
//! ```
//! use inotify::{
//!     Inotify,
//!     WatchMask,
//! };
//!
//! let mut inotify = Inotify::init()
//!     .expect("Error while initializing inotify instance");
//!
//! # // Create a temporary file, so `add_watch` won't return an error.
//! # use std::fs::File;
//! # let mut test_file = File::create("/tmp/inotify-rs-test-file")
//! #     .expect("Failed to create test file");
//! #
//! // Watch for modify and close events.
//! inotify
//!     .add_watch(
//!         "/tmp/inotify-rs-test-file",
//!         WatchMask::MODIFY | WatchMask::CLOSE,
//!     )
//!     .expect("Failed to add file watch");
//!
//! # // Modify file, so the following `read_events_blocking` won't block.
//! # use std::io::Write;
//! # write!(&mut test_file, "something\n")
//! #     .expect("Failed to write something to test file");
//! #
//! // Read events that were added with `add_watch` above.
//! let mut buffer = [0; 1024];
//! let events = inotify.read_events_blocking(&mut buffer)
//!     .expect("Error while reading events");
//!
//! for event in events {
//!     // Handle event
//! }
//! ```
//!
//! # Attention: inotify gotchas
//!
//! inotify (as in, the Linux API, not this wrapper) has many edge cases, making
//! it hard to use correctly. This can lead to weird and hard to find bugs in
//! applications that are based on it. inotify-rs does its best to fix these
//! issues, but sometimes this would require an amount of runtime overhead that
//! is just unacceptable for a low-level wrapper such as this.
//!
//! We've documented any issues that inotify-rs has inherited from inotify, as
//! far as we are aware of them. Please watch out for any further warnings
//! throughout this documentation. If you want to be on the safe side, in case
//! we have missed something, please read the [inotify man pages] carefully.
//!
//! [inotify-rs]: https://crates.io/crates/inotify
//! [inotify]: https://en.wikipedia.org/wiki/Inotify
//! [`Inotify`]: struct.Inotify.html
//! [inotify man pages]: http://man7.org/linux/man-pages/man7/inotify.7.html


#[macro_use]
extern crate bitflags;

extern crate libc;
extern crate inotify_sys as ffi;


use std::mem;
use std::hash::{
    Hash,
    Hasher,
};
use std::io;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{
    AsRawFd,
    FromRawFd,
    IntoRawFd,
    RawFd,
};
use std::path::Path;
use std::rc::{
    Rc,
    Weak,
};
use std::slice;
use std::ffi::{
    OsStr,
    CString,
};

use libc::{
    F_GETFL,
    F_SETFL,
    O_NONBLOCK,
    fcntl,
    c_void,
    size_t,
    c_int,
};


/// Idiomatic Rust wrapper around Linux's inotify API
///
/// `Inotify` is a wrapper around an inotify instance. It generally tries to
/// adhere to the underlying inotify API closely, while making access to it
/// safe and convenient.
///
/// Please refer to the [top-level documentation] for further details and a
/// usage example.
///
/// [top-level documentation]: index.html
pub struct Inotify {
    fd           : Rc<RawFd>,
    close_on_drop: bool,
}

impl Inotify {
    /// Creates an [`Inotify`] instance
    ///
    /// Initializes an inotify instance by calling [`inotify_init1`].
    ///
    /// This method passes both flags accepted by [`inotify_init1`], not giving
    /// the user any choice in the matter, as not passing the flags would be
    /// inappropriate in the context of this wrapper:
    ///
    /// - [`IN_CLOEXEC`] prevents leaking file descriptors to other processes.
    /// - [`IN_NONBLOCK`] controls the blocking behavior of the inotify API,
    ///   which is entirely managed by this wrapper.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_init1`], without
    /// adding any error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    /// ```
    ///
    /// [`Inotify`]: struct.Inotify.html
    /// [`inotify_init1`]: ../inotify_sys/fn.inotify_init1.html
    /// [`IN_CLOEXEC`]: ../inotify_sys/constant.IN_CLOEXEC.html
    /// [`IN_NONBLOCK`]: ../inotify_sys/constant.IN_NONBLOCK.html
    pub fn init() -> io::Result<Inotify> {
        let fd = unsafe {
            // Initialize inotify and pass both `IN_CLOEXEC` and `IN_NONBLOCK`.
            //
            // `IN_NONBLOCK` is needed, because `Inotify` manages blocking
            // behavior for the API consumer, and the way we do that is to make
            // everything non-blocking by default and later override that as
            // required.
            //
            // Passing `IN_CLOEXEC` prevents leaking file descriptors to
            // processes executed by this process and seems to be a best
            // practice. I don't grasp this issue completely and failed to find
            // any authorative sources on the topic. There's some discussion in
            // the open(2) and fcntl(2) man pages, but I didn't find that
            // helpful in understanding the issue of leaked file scriptors.
            // For what it's worth, there's a Rust issue about this:
            // https://github.com/rust-lang/rust/issues/12148
            ffi::inotify_init1(ffi::IN_CLOEXEC | ffi::IN_NONBLOCK)
        };

        match fd {
            -1 => Err(io::Error::last_os_error()),
            _  =>
                Ok(Inotify {
                    fd           : Rc::new(fd),
                    close_on_drop: true,
                }),
        }
    }

    /// Adds or updates a watch for the given path
    ///
    /// Adds a new watch or updates an existing one for the file referred to by
    /// `path`. Returns a watch descriptor that can be used to refer to this
    /// watch later.
    ///
    /// The `mask` argument defines what kind of changes the file should be
    /// watched for, and how to do that. See the documentation of [`WatchMask`]
    /// for details.
    ///
    /// If this method is used to add a new watch, a new [`WatchDescriptor`] is
    /// returned. If it is used to update an existing watch, a
    /// [`WatchDescriptor`] that equals the previously returned
    /// [`WatchDescriptor`] for that watch is returned intead.
    ///
    /// Under the hood, this method just calls [`inotify_add_watch`] and does
    /// some trivial translation between the types on the Rust side and the C
    /// side.
    ///
    /// # Attention: Updating watches and hardlinks
    ///
    /// As mentioned above, this method can be used to update an existing watch.
    /// This is usually done by calling this method with the same `path`
    /// argument that it has been called with before. But less obviously, it can
    /// also happen if the method is called with a different path that happens
    /// to link to the same inode.
    ///
    /// You can detect this by keeping track of [`WatchDescriptor`]s and the
    /// paths they have been returned for. If the same [`WatchDescriptor`] is
    /// returned for a different path (and you haven't freed the
    /// [`WatchDescriptor`] by removing the watch), you know you have two paths
    /// pointing to the same inode, being watched by the same watch.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to
    /// [`inotify_add_watch`][`inotify_add_watch`] (translated into an
    /// `io::Error`), without adding any error conditions of
    /// its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::{
    ///     Inotify,
    ///     WatchMask,
    /// };
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::MODIFY)
    ///     .expect("Failed to add file watch");
    ///
    /// // Handle events for the file here
    /// ```
    ///
    /// [`inotify_add_watch`]: ../inotify_sys/fn.inotify_add_watch.html
    /// [`WatchMask`]: struct.WatchMask.html
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    pub fn add_watch<P>(&mut self, path: P, mask: WatchMask)
        -> io::Result<WatchDescriptor>
        where P: AsRef<Path>
    {
        let path = CString::new(path.as_ref().as_os_str().as_bytes())?;

        let wd = unsafe {
            ffi::inotify_add_watch(
                *self.fd,
                path.as_ptr() as *const _,
                mask.bits(),
            )
        };

        match wd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(WatchDescriptor{ id: wd, fd: Rc::downgrade(&self.fd) }),
        }
    }

    /// Stops watching a file
    ///
    /// Removes the watch represented by the provided [`WatchDescriptor`] by
    /// calling [`inotify_rm_watch`]. [`WatchDescriptor`]s can be obtained via
    /// [`Inotify::add_watch`], or from the `wd` field of [`Event`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_rm_watch`].
    /// Returns an [`io::Error`] with [`ErrorKind`]`::InvalidInput`, if the given
    /// [`WatchDescriptor`] did not originate from this [`Inotify`] instance.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # let mut test_file = File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// # // Add a watch and modify the file, so the code below doesn't block
    /// # // forever.
    /// # use inotify::WatchMask;
    /// # inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::MODIFY)
    /// #     .expect("Failed to add file watch");
    /// # use std::io::Write;
    /// # write!(&mut test_file, "something\n")
    /// #     .expect("Failed to write something to test file");
    /// #
    /// let mut buffer = [0; 1024];
    /// let events = inotify
    ///     .read_events_blocking(&mut buffer)
    ///     .expect("Error while waiting for events");
    ///
    /// for event in events {
    ///     inotify.rm_watch(event.wd);
    /// }
    /// ```
    ///
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    /// [`inotify_rm_watch`]: ../inotify_sys/fn.inotify_rm_watch.html
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    /// [`Event`]: struct.Event.html
    /// [`Inotify`]: struct.Inotify.html
    /// [`io::Error`]: https://doc.rust-lang.org/std/io/struct.Error.html
    /// [`ErrorKind`]: https://doc.rust-lang.org/std/io/enum.ErrorKind.html
    pub fn rm_watch(&mut self, wd: WatchDescriptor) -> io::Result<()> {
        if wd.fd.upgrade().as_ref() != Some(&self.fd) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Invalid WatchDescriptor",
            ));
        }

        let result = unsafe { ffi::inotify_rm_watch(*self.fd, wd.id) };
        match result {
            0  => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _  => panic!(
                "unexpected return code from inotify_rm_watch ({})", result)
        }
    }

    /// Waits until events are available, then returns them
    ///
    /// This method will block the current thread until at least one event is
    /// available. If this is not desirable, please consider [`read_events`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`read`], without adding any
    /// error conditions of its own.
    ///
    /// [`read_events`]: struct.Inotify.html#method.read_events
    /// [`read`]: ../libc/fn.read.html
    pub fn read_events_blocking<'a>(&mut self, buffer: &'a mut [u8])
        -> io::Result<Events<'a>>
    {
        unsafe {
            fcntl(*self.fd, F_SETFL, fcntl(*self.fd, F_GETFL) & !O_NONBLOCK)
        };
        let result = self.read_events(buffer);
        unsafe {
            fcntl(*self.fd, F_SETFL, fcntl(*self.fd, F_GETFL) | O_NONBLOCK)
        };

        result
    }

    /// Returns any available events
    ///
    /// Returns an iterator over all events that are currently available. If no
    /// events are available, an iterator is still returned.
    ///
    /// The `buffer` argument, as the name indicates, is used as a buffer for
    /// the inotify events. Its contents may be overwritten.
    ///
    /// If you need a method that will block until at least one event is
    /// available, please consider [`read_events_blocking`].
    ///
    /// # Errors
    ///
    /// This function directly returns all errors from the call to [`read`]. In
    /// addition, [`ErrorKind`]`::UnexpectedEof` is returned, if the call to
    /// [`read`] returns `0`, signaling end-of-file.
    ///
    /// If `buffer` is too small, this will result in an error with
    /// [`ErrorKind`]`::InvalidInput`. On very old Linux kernels,
    /// [`ErrorKind`]`::UnexpectedEof` will be returned instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// let mut buffer = [0; 1024];
    /// let events = inotify.read_events(&mut buffer)
    ///     .expect("Error while reading events");
    ///
    /// for event in events {
    ///     // Handle event
    /// }
    /// ```
    ///
    /// [`read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
    /// [`read`]: ../libc/fn.read.html
    /// [`ErrorKind`]: https://doc.rust-lang.org/std/io/enum.ErrorKind.html
    pub fn read_events<'a>(&mut self, buffer: &'a mut [u8])
        -> io::Result<Events<'a>>
    {
        let num_bytes = unsafe {
            ffi::read(
                *self.fd,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as size_t
            )
        };

        let num_bytes = match num_bytes {
            0 => {
                return Err(
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "`read` return `0`, signaling end-of-file"
                    )
                );
            }
            -1 => {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(Events::new(Rc::downgrade(&self.fd), buffer, 0));
                }
                else {
                    return Err(error);
                }
            },
            _ if num_bytes < 0 => {
                panic!("{} {} {} {} {} {}",
                    "Unexpected return value from `read`. Received a negative",
                    "value that was not `-1`. According to the `read` man page",
                    "this shouldn't happen, as either `-1` is returned on",
                    "error, `0` on end-of-file, or a positive value for the",
                    "number of bytes read. Returned value:",
                    num_bytes,
                );
            }
            _ => {
                // The value returned by `read` should be `isize`. Let's quickly
                // verify this with the following assignment, so we can be sure
                // our cast below is valid.
                let num_bytes: isize = num_bytes;

                // The type returned by `read` is `isize`, and we've ruled out
                // all negative values with the match arms above. This means we
                // can safely cast to `usize`.
                debug_assert!(num_bytes > 0);
                num_bytes as usize
            }
        };

        Ok(Events::new(Rc::downgrade(&self.fd), buffer, num_bytes))
    }

    /// Closes the inotify instance
    ///
    /// Closes the file descriptor referring to the inotify instance. The user
    /// usually doesn't have to call this function, as the underlying inotify
    /// instance is closed automatically, when [`Inotify`] is dropped.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`close`], without adding any
    /// error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// inotify.close()
    ///     .expect("Failed to close inotify instance");
    /// ```
    ///
    /// [`Inotify`]: struct.Inotify.html
    /// [`close`]: ../libc/fn.close.html
    pub fn close(mut self) -> io::Result<()> {
        // `self` will be dropped when this method returns. The `Drop`
        // implementation will attempt to close the file descriptor again,
        // unless this flag here is cleared.
        self.close_on_drop = false;

        match unsafe { ffi::close(*self.fd) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        if self.close_on_drop {
            unsafe { ffi::close(*self.fd); }
        }
    }
}

impl AsRawFd for Inotify {
    fn as_raw_fd(&self) -> RawFd {
        *self.fd
    }
}

impl FromRawFd for Inotify {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Inotify {
            fd           : Rc::new(fd),
            close_on_drop: true,
        }
    }
}

impl IntoRawFd for Inotify {
    fn into_raw_fd(mut self) -> RawFd {
        self.close_on_drop = false;
        *self.fd
    }
}


bitflags! {
    /// Describes a file system watch
    ///
    /// Passed to [`Inotify::add_watch`], to describe what file system events
    /// to watch for, and how to do that.
    ///
    /// # Examples
    ///
    /// `WatchMask` constants can be passed to [`Inotify::add_watch`] as is. For
    /// example, here's how to create a watch that triggers an event when a file
    /// is accessed:
    ///
    /// ``` rust
    /// # use inotify::{
    /// #     Inotify,
    /// #     WatchMask,
    /// # };
    /// #
    /// # let mut inotify = Inotify::init().unwrap();
    /// #
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::ACCESS)
    ///    .expect("Error adding watch");
    /// ```
    ///
    /// You can also combine multiple `WatchMask` constants. Here we add a watch
    /// this is triggered both when files are created or deleted in a directory:
    ///
    /// ``` rust
    /// # use inotify::{
    /// #     Inotify,
    /// #     WatchMask,
    /// # };
    /// #
    /// # let mut inotify = Inotify::init().unwrap();
    /// inotify.add_watch("/tmp/", WatchMask::CREATE | WatchMask::DELETE)
    ///    .expect("Error adding watch");
    /// ```
    ///
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    pub struct WatchMask: u32 {
        /// File was accessed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_ACCESS`].
        ///
        /// [`inotify_sys::IN_ACCESS`]: ../inotify_sys/constant.IN_ACCESS.html
        const ACCESS = ffi::IN_ACCESS;

        /// Metadata (permissions, timestamps, ...) changed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_ATTRIB`].
        ///
        /// [`inotify_sys::IN_ATTRIB`]: ../inotify_sys/constant.IN_ATTRIB.html
        const ATTRIB = ffi::IN_ATTRIB;

        /// File opened for writing was closed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CLOSE_WRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_WRITE`]: ../inotify_sys/constant.IN_CLOSE_WRITE.html
        const CLOSE_WRITE = ffi::IN_CLOSE_WRITE;

        /// File or directory not opened for writing was closed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_CLOSE_NOWRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_NOWRITE`]: ../inotify_sys/constant.IN_CLOSE_NOWRITE.html
        const CLOSE_NOWRITE = ffi::IN_CLOSE_NOWRITE;

        /// File/directory created in watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CREATE`].
        ///
        /// [`inotify_sys::IN_CREATE`]: ../inotify_sys/constant.IN_CREATE.html
        const CREATE = ffi::IN_CREATE;

        /// File/directory deleted from watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_DELETE`].
        ///
        /// [`inotify_sys::IN_DELETE`]: ../inotify_sys/constant.IN_DELETE.html
        const DELETE = ffi::IN_DELETE;

        /// Watched file/directory was deleted
        ///
        /// See [`inotify_sys::IN_DELETE_SELF`].
        ///
        /// [`inotify_sys::IN_DELETE_SELF`]: ../inotify_sys/constant.IN_DELETE_SELF.html
        const DELETE_SELF = ffi::IN_DELETE_SELF;

        /// File was modified
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MODIFY`].
        ///
        /// [`inotify_sys::IN_MODIFY`]: ../inotify_sys/constant.IN_MODIFY.html
        const MODIFY = ffi::IN_MODIFY;

        /// Watched file/directory was moved
        ///
        /// See [`inotify_sys::IN_MOVE_SELF`].
        ///
        /// [`inotify_sys::IN_MOVE_SELF`]: ../inotify_sys/constant.IN_MOVE_SELF.html
        const MOVE_SELF = ffi::IN_MOVE_SELF;

        /// File was renamed/moved; watched directory contained old name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_FROM`].
        ///
        /// [`inotify_sys::IN_MOVED_FROM`]: ../inotify_sys/constant.IN_MOVED_FROM.html
        const MOVED_FROM = ffi::IN_MOVED_FROM;

        /// File was renamed/moved; watched directory contains new name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_TO`].
        ///
        /// [`inotify_sys::IN_MOVED_TO`]: ../inotify_sys/constant.IN_MOVED_TO.html
        const MOVED_TO = ffi::IN_MOVED_TO;

        /// File or directory was opened
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_OPEN`].
        ///
        /// [`inotify_sys::IN_OPEN`]: ../inotify_sys/constant.IN_OPEN.html
        const OPEN = ffi::IN_OPEN;

        /// Watch for all events
        ///
        /// This constant is simply a conventient combination of the following
        /// other constants:
        ///
        /// - [`ACCESS`]
        /// - [`ATTRIB`]
        /// - [`CLOSE_WRITE`]
        /// - [`CLOSE_NOWRITE`]
        /// - [`CREATE`]
        /// - [`DELETE`]
        /// - [`DELETE_SELF`]
        /// - [`MODIFY`]
        /// - [`MOVE_SELF`]
        /// - [`MOVED_FROM`]
        /// - [`MOVED_TO`]
        /// - [`OPEN`]
        ///
        /// See [`inotify_sys::IN_ALL_EVENTS`].
        ///
        /// [`ACCESS`]: #associatedconstant.ACCESS
        /// [`ATTRIB`]: #associatedconstant.ATTRIB
        /// [`CLOSE_WRITE`]: #associatedconstant.CLOSE_WRITE
        /// [`CLOSE_NOWRITE`]: #associatedconstant.CLOSE_NOWRITE
        /// [`CREATE`]: #associatedconstant.CREATE
        /// [`DELETE`]: #associatedconstant.DELETE
        /// [`DELETE_SELF`]: #associatedconstant.DELETE_SELF
        /// [`MODIFY`]: #associatedconstant.MODIFY
        /// [`MOVE_SELF`]: #associatedconstant.MOVE_SELF
        /// [`MOVED_FROM`]: #associatedconstant.MOVED_FROM
        /// [`MOVED_TO`]: #associatedconstant.MOVED_TO
        /// [`OPEN`]: #associatedconstant.OPEN
        /// [`inotify_sys::IN_ALL_EVENTS`]: ../inotify_sys/constant.IN_ALL_EVENTS.html
        const ALL_EVENTS = ffi::IN_ALL_EVENTS;

        /// Watch for all move events
        ///
        /// This constant is simply a conventient combination of the following
        /// other constants:
        ///
        /// - [`MOVED_FROM`]
        /// - [`MOVED_TO`]
        ///
        /// See [`inotify_sys::IN_MOVE`].
        ///
        /// [`MOVED_FROM`]: #associatedconstant.MOVED_FROM
        /// [`MOVED_TO`]: #associatedconstant.MOVED_TO
        /// [`inotify_sys::IN_MOVE`]: ../inotify_sys/constant.IN_MOVE.html
        const MOVE = ffi::IN_MOVE;

        /// Watch for all close events
        ///
        /// This constant is simply a conventient combination of the following
        /// other constants:
        ///
        /// - [`CLOSE_WRITE`]
        /// - [`CLOSE_NOWRITE`]
        ///
        /// See [`inotify_sys::IN_CLOSE`].
        ///
        /// [`CLOSE_WRITE`]: #associatedconstant.CLOSE_WRITE
        /// [`CLOSE_NOWRITE`]: #associatedconstant.CLOSE_NOWRITE
        /// [`inotify_sys::IN_CLOSE`]: ../inotify_sys/constant.IN_CLOSE.html
        const CLOSE = ffi::IN_CLOSE;

        /// Don't dereference the path if it is a symbolic link
        ///
        /// See [`inotify_sys::IN_DONT_FOLLOW`].
        ///
        /// [`inotify_sys::IN_DONT_FOLLOW`]: ../inotify_sys/constant.IN_DONT_FOLLOW.html
        const DONT_FOLLOW = ffi::IN_DONT_FOLLOW;

        /// Filter events for directory entries that have been unlinked
        ///
        /// See [`inotify_sys::IN_EXCL_UNLINK`].
        ///
        /// [`inotify_sys::IN_EXCL_UNLINK`]: ../inotify_sys/constant.IN_EXCL_UNLINK.html
        const EXCL_UNLINK = ffi::IN_EXCL_UNLINK;

        /// If a watch for the inode exists, amend it instead of replacing it
        ///
        /// See [`inotify_sys::IN_MASK_ADD`].
        ///
        /// [`inotify_sys::IN_MASK_ADD`]: ../inotify_sys/constant.IN_MASK_ADD.html
        const MASK_ADD = ffi::IN_MASK_ADD;

        /// Only receive one event, then remove the watch
        ///
        /// See [`inotify_sys::IN_ONESHOT`].
        ///
        /// [`inotify_sys::IN_ONESHOT`]: ../inotify_sys/constant.IN_ONESHOT.html
        const ONESHOT = ffi::IN_ONESHOT;

        /// Only watch path, if it is a directory
        ///
        /// See [`inotify_sys::IN_ONLYDIR`].
        ///
        /// [`inotify_sys::IN_ONLYDIR`]: ../inotify_sys/constant.IN_ONLYDIR.html
        const ONLYDIR = ffi::IN_ONLYDIR;
    }
}


/// Represents a watch on an inode
///
/// Can be obtained from [`Inotify::add_watch`] or from an [`Event`]. A watch
/// descriptor can be used to get inotify to stop watching an inode by passing
/// it to [`Inotify::rm_watch`].
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
/// [`Event`]: struct.Event.html
#[derive(Clone, Debug)]
pub struct WatchDescriptor{
    id: c_int,
    fd: Weak<RawFd>,
}

impl Eq for WatchDescriptor {}

impl PartialEq for WatchDescriptor {
    fn eq(&self, other: &Self) -> bool {
        let self_fd  = self.fd.upgrade();
        let other_fd = other.fd.upgrade();

        self.id == other.id && self_fd.is_some() && self_fd == other_fd
    }
}

impl Hash for WatchDescriptor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // This function only takes `self.id` into account, as `self.fd` is a
        // weak pointer that might no longer be available. Since neither
        // panicking nor changing the hash depending on whether it's available
        // is acceptable, we just don't look at it at all.
        // I don't think that this influences storage in a `HashMap` or
        // `HashSet` negatively, as storing `WatchDescriptor`s from different
        // `Inotify` instances seems like something of an antipattern anyway.
        self.id.hash(state);
    }
}


/// Iterator over inotify events
///
/// Allows for iteration over the events returned by
/// [`Inotify::read_events_blocking`] or [`Inotify::read_events`].
///
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
pub struct Events<'a> {
    fd       : Weak<RawFd>,
    buffer   : &'a [u8],
    num_bytes: usize,
    pos      : usize,
}

impl<'a> Events<'a> {
    fn new(fd: Weak<RawFd>, buffer: &'a [u8], num_bytes: usize) -> Self {
        Events {
            fd       : fd,
            buffer   : buffer,
            num_bytes: num_bytes,
            pos      : 0,
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let event_size = mem::size_of::<ffi::inotify_event>();

        if self.pos < self.num_bytes {
            // `self.buffer` contains the data that was read from the inotify
            // instance. `self.num_bytes` is the number of bytes that were read.
            // And as per the if condition above, `self.pos < self.num_bytes`,
            // so our current position is still within the bounds of the buffer.
            // This means, unless inotify lied to us, or we did something
            // horribly wrong, there should be at least another event worth of
            // bytes in the buffer.
            debug_assert!(self.num_bytes - self.pos >= event_size);

            let slice = &self.buffer[self.pos..];
            let event = slice.as_ptr() as *const ffi::inotify_event;

            // We have a pointer to an `inotify_event` that points into the
            // buffer at offset `self.pos`. Since we know, as per the assertion
            // above, that there are enough bytes for at least one more event in
            // the buffer, dereferencing that pointer is safe.
            let event = unsafe { *event };

            // The call to `offset` is safe, as long as the starting and the
            // resulting pointer are either in bounds or one byte past the end
            // of an allocated object. As we've established above, there are
            // enough bytes for the `inotify_event` left in the buffer. If there
            // is anything else in the buffer, the new pointer will be within an
            // allocated object. If there is nothing else, it will be exactly
            // one byte past it.
            let name = unsafe {
                slice
                    .as_ptr()
                    .offset(event_size as isize)
            };

            // Right behind the `inotify_event` struct is the event's name. The
            // name's length is given by `event.len`. There should always be
            // enough bytes left in the buffer to fit the name.
            let name_pos = self.pos + event_size;
            debug_assert!(self.num_bytes - name_pos >= event.len as usize);

            // As we've established above, the name fits within the buffer. This
            // means that there's either an actual name in there, with enough
            // bytes to make the created slice valid, or `event.len` is `0`, in
            // which case the function call is safe in any case, as long as
            // `name` is not null. We know it's not, because we created it from
            // a slice right above.
            let name = unsafe {
                slice::from_raw_parts(
                    name,
                    event.len as usize,
                )
            };

            // Remove trailing \0 bytes
            //
            // The events in the buffer are aligned, and `name` is filled up
            // with '\0' up to the alignment boundary. Here we remove those
            // additional bytes.
            //
            // The `unwrap` here is safe, because `splitn` always returns at
            // least one result, even if the original slice contains no '\0'.
            let name = name
                .splitn(2, |b| b == &0u8)
                .next()
                .unwrap();

            self.pos += event_size + event.len as usize;

            Some(Event::new(
                self.fd.clone(),
                &event,
                OsStr::from_bytes(name),
            ))
        }
        else {
            None
        }
    }
}


/// An inotify event
///
/// A file system event that describes a change that the user previously
/// registered interest in. To watch for events, call [`Inotify::add_watch`]. To
/// retrieve events, call [`Inotify::read_events_blocking`] or
/// [`Inotify::read_events`].
///
/// # Examples
///
/// Here's how to determine if this event indicates that a file was modified:
///
/// ``` rust
/// # use std::mem;
/// # use std::os::unix::io::RawFd;
/// # use std::rc::Weak;
/// #
/// # use inotify::{
/// #     Event,
/// #     EventMask,
/// # };
/// #
/// # // Construct a fake event for the sake of this example
/// # let event: Event = Event {
/// #     wd    : unsafe { mem::transmute((0, Weak::<RawFd>::new())) },
/// #     mask  : EventMask::MODIFY,
/// #     cookie: 0,
/// #     name  : None,
/// # };
/// #
/// if event.mask.contains(EventMask::MODIFY) {
///     // do something
/// }
/// ```
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
#[derive(Clone, Debug)]
pub struct Event<'a> {
    /// Identifies the watch this event originates from
    ///
    /// This [`WatchDescriptor`] is equal to the one that [`Inotify::add_watch`]
    /// returned when interest for this event was registered. The
    /// [`WatchDescriptor`] can be used to remove the watch using
    /// [`Inotify::rm_watch`], thereby preventing future events of this type
    /// from being created.
    ///
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    /// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
    pub wd    : WatchDescriptor,

    /// Indicates what kind of event this is
    pub mask  : EventMask,

    /// Connects related events to each other
    ///
    /// When a file is renamed, this results two events: [`MOVED_FROM`] and
    /// [`MOVED_TO`]. The `cookie` field will be the same for both of them,
    /// thereby making is possible to connect the event pair.
    ///
    /// [`MOVED_FROM`]: event_mask/constant.MOVED_FROM.html
    /// [`MOVED_TO`]: event_mask/constant.MOVED_TO.html
    pub cookie: u32,

    /// The name of the file the event originates from
    ///
    /// This field is set only, if the subject of the event is a file in a
    /// wacthed directory. If the event concerns a file or directory that is
    /// watched directly, `name` will be `None`.
    pub name  : Option<&'a OsStr>,
}

impl<'a> Event<'a> {
    fn new(fd: Weak<RawFd>, event: &ffi::inotify_event, name: &'a OsStr)
        -> Self
    {
        let mask = EventMask::from_bits(event.mask)
            .expect("Failed to convert event mask. This indicates a bug.");

        let wd = ::WatchDescriptor {
            id: event.wd,
            fd: fd,
        };

        let name = if name == "" {
            None
        }
        else {
            Some(name)
        };

        Event {
            wd    : wd,
            mask  : mask,
            cookie: event.cookie,
            name  : name,
        }
    }
}


bitflags! {
    /// Mask for an event
    ///
    /// This struct can be retrieved from an [`Event`] via its `mask` field.
    /// You can determine the [`Event`]'s type by comparing it to the
    /// constants in [this module] module using [`EventMask::contains`].
    ///
    /// [`Event`]: ../struct.Event.html
    /// [this module]: index.html
    /// [`EventMask::contains`]: struct.EventMask.html#method.contains
    pub struct EventMask: u32 {
        /// File was accessed.
        const ACCESS = ffi::IN_ACCESS;

        /// Metadata changed.
        const ATTRIB = ffi::IN_ATTRIB;

        /// File opened for writing was closed.
        const CLOSE_WRITE = ffi::IN_CLOSE_WRITE;

        /// File or directory not opened for writing was closed.
        const CLOSE_NOWRITE = ffi::IN_CLOSE_NOWRITE;

        /// File/directory created in watched directory.
        const CREATE = ffi::IN_CREATE;

        /// File/directory deleted from watched directory.
        const DELETE = ffi::IN_DELETE;

        /// Watched file/directory was itself deleted.
        const DELETE_SELF = ffi::IN_DELETE_SELF;

        /// File was modified.
        const MODIFY = ffi::IN_MODIFY;

        /// Watched file/directory was itself moved.
        const MOVE_SELF = ffi::IN_MOVE_SELF;

        /// Generated for the directory containing the old filename when a
        /// file is renamend.
        const MOVED_FROM = ffi::IN_MOVED_FROM;

        /// Generated for the directory containing the new filename when a
        /// file is renamed.
        const MOVED_TO = ffi::IN_MOVED_TO;

        /// File or directory was opened.
        const OPEN = ffi::IN_OPEN;

        /// Watch was removed.
        const IGNORED = ffi::IN_IGNORED;

        /// Subject of this event is a directory.
        const ISDIR = ffi::IN_ISDIR;

        /// Event queue overflowed.
        const Q_OVERFLOW = ffi::IN_Q_OVERFLOW;

        /// File system containing watched object was unmounted.
        const UNMOUNT = ffi::IN_UNMOUNT;
    }
}
