use std::{
    ffi::{
        OsStr,
        OsString,
    },
    mem,
    os::unix::ffi::OsStrExt,
    slice,
    sync::Weak,
};

use inotify_sys as ffi;

use fd_guard::FdGuard;
use watches::WatchDescriptor;


/// Iterator over inotify events
///
/// Allows for iteration over the events returned by
/// [`Inotify::read_events_blocking`] or [`Inotify::read_events`].
///
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
pub struct Events<'a> {
    fd       : Weak<FdGuard>,
    buffer   : &'a [u8],
    num_bytes: usize,
    pos      : usize,
}

impl<'a> Events<'a> {
    pub(crate) fn new(fd: Weak<FdGuard>, buffer: &'a [u8], num_bytes: usize)
        -> Self
    {
        Events {
            fd       : fd,
            buffer   : buffer,
            num_bytes: num_bytes,
            pos      : 0,
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Event<&'a OsStr>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.num_bytes {
            let (step, event) = Event::from_buffer(self.fd.clone(), &self.buffer[self.pos..], self.num_bytes - self.pos);
            self.pos += step;

            Some(event)
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
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
#[derive(Clone, Debug)]
pub struct Event<S> {
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
    pub wd: WatchDescriptor,

    /// Indicates what kind of event this is
    pub mask: EventMask,

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
    /// This field is set only if the subject of the event is a file in a
    /// watched directory. If the event concerns a file or directory that is
    /// watched directly, `name` will be `None`.
    pub name: Option<S>,
}

impl<'a> Event<&'a OsStr> {
    fn new(fd: Weak<FdGuard>, event: &ffi::inotify_event, name: &'a OsStr)
        -> Self
    {
        let mask = EventMask::from_bits(event.mask)
            .expect("Failed to convert event mask. This indicates a bug.");

        let wd = ::WatchDescriptor {
            id: event.wd,
            fd,
        };

        let name = if name == "" {
            None
        }
        else {
            Some(name)
        };

        Event {
            wd,
            mask,
            cookie: event.cookie,
            name,
        }
    }

    pub(crate) fn from_buffer(
        fd       : Weak<FdGuard>,
        buffer   : &'a [u8],
        num_bytes: usize
    )
        -> (usize, Self)
    {
        let event_size = mem::size_of::<ffi::inotify_event>();

        // `self.buffer` contains the data that was read from the inotify
        // instance. `self.num_bytes` is the number of bytes that were read.
        // And as per the if condition above, `self.pos < self.num_bytes`,
        // so our current position is still within the bounds of the buffer.
        // This means, unless inotify lied to us, or we did something
        // horribly wrong, there should be at least another event worth of
        // bytes in the buffer.
        debug_assert!(num_bytes >= event_size);

        let slice = &buffer[..];
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
        let name_pos = event_size;
        debug_assert!(num_bytes - name_pos >= event.len as usize);

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

        let step = event_size + event.len as usize;
        let event = Event::new(
            fd,
            &event,
            OsStr::from_bytes(name),
        );

        (step, event)
    }

    pub(crate) fn into_owned(&self) -> EventOwned {
        Event {
            wd: self.wd.clone(),
            mask: self.mask,
            cookie: self.cookie,
            name: self.name.map(OsStr::to_os_string),
        }
    }
}


/// An owned version of `Event`
pub type EventOwned = Event<OsString>;


bitflags! {
    /// Indicates the type of an event
    ///
    /// This struct can be retrieved from an [`Event`] via its `mask` field.
    /// You can determine the [`Event`]'s type by comparing the `EventMask` to
    /// its associated constants.
    ///
    /// Please refer to the documentation of [`Event`] for a usage example.
    ///
    /// [`Event`]: struct.Event.html
    pub struct EventMask: u32 {
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

        /// Watch was removed
        ///
        /// This event will be generated, if the watch was removed explicitly
        /// (via [`Inotify::rm_watch`]), or automatically (because the file was
        /// deleted or the file system was unmounted).
        ///
        /// See [`inotify_sys::IN_IGNORED`].
        ///
        /// [`inotify_sys::IN_IGNORED`]: ../inotify_sys/constant.IN_IGNORED.html
        const IGNORED = ffi::IN_IGNORED;

        /// Event related to a directory
        ///
        /// The subject of the event is a directory.
        ///
        /// See [`inotify_sys::IN_ISDIR`].
        ///
        /// [`inotify_sys::IN_ISDIR`]: ../inotify_sys/constant.IN_ISDIR.html
        const ISDIR = ffi::IN_ISDIR;

        /// Event queue overflowed
        ///
        /// The event queue has overflowed and events have presumably been lost.
        ///
        /// See [`inotify_sys::IN_Q_OVERFLOW`].
        ///
        /// [`inotify_sys::IN_Q_OVERFLOW`]: ../inotify_sys/constant.IN_Q_OVERFLOW.html
        const Q_OVERFLOW = ffi::IN_Q_OVERFLOW;

        /// File system containing watched object was unmounted.
        /// File system was unmounted
        ///
        /// The file system that contained the watched object has been
        /// unmounted. An event with [`WatchMask::IGNORED`] will subsequently be
        /// generated for the same watch descriptor.
        ///
        /// See [`inotify_sys::IN_UNMOUNT`].
        ///
        /// [`WatchMask::IGNORED`]: #associatedconstant.IGNORED
        /// [`inotify_sys::IN_UNMOUNT`]: ../inotify_sys/constant.IN_UNMOUNT.html
        const UNMOUNT = ffi::IN_UNMOUNT;
    }
}
