#![allow(missing_docs)]

//! Idiomatic wrapper for inotify

use epoll::{self, EpollEvent};
use epoll::util::ctl_op as EpollControlOp;
use epoll::util::event_type as EpollEventType;
use libc::{
    F_GETFL,
    F_SETFL,
    O_NONBLOCK,
    fcntl,
    c_int,
    c_void,
    size_t,
    ssize_t
};
use std::ffi::{
    CString,
};
use std::mem;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::slice;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use ffi;
use ffi::inotify_event;

pub type Watch = c_int;

pub struct INotify {
    fd: Option<c_int>,
    events: Vec<Event>,
    closer: Option<Arc<INotifyCloser>>,
}

const OPEN: usize         = 0b0000;
const CLOSING: usize      = 0b0001;
const CLOSED: usize       = 0b0010;
const CLOSE_FAILED: usize = 0b0100;

pub struct INotifyCloser {
    state: AtomicUsize,
    mutex: Mutex<()>,
    cvar: Condvar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum INotifyState {
    Open,
    Closing,
    Closed,
    CloseFailed,
}

impl INotify {
    pub fn init() -> io::Result<INotify> {
        INotify::init_with_flags(0)
    }

    pub fn init_with_flags(flags: isize) -> io::Result<INotify> {
        let fd = unsafe { ffi::inotify_init1(flags as c_int) };

        unsafe { fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK) };

        match fd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(INotify {
                fd    : Some(fd),
                events: Vec::new(),
                closer: None,
            })
        }
    }

    pub fn add_watch(&mut self, path: &Path, mask: u32) -> io::Result<Watch> {
        let fd = self.fd.expect("The inotify handler was already closed.");

        let wd = unsafe {
            let c_str = try!(CString::new(path.as_os_str().as_bytes()));

            ffi::inotify_add_watch(
                fd,
                c_str.as_ptr(),
                mask
            )
        };

        match wd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(wd)
        }
    }

    pub fn rm_watch(&mut self, watch: Watch) -> io::Result<()> {
        let fd = self.fd.expect("The inotify handler was already closed.");

        let result = unsafe { ffi::inotify_rm_watch(fd, watch) };

        match result {
            0  => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _  => panic!(
                "unexpected return code from inotify_rm_watch ({})", result)
        }
    }

    /// Wait until events are available, then return them.
    /// This function will block until events are available or return an
    /// empty slice iff the inotify object was closed. If you want
    /// non-blocking behavior, use `available_events`.
    pub fn wait_for_events(&mut self) -> io::Result<&[Event]> {
        let state = self.state();
        match state {
            INotifyState::Open => self.handle_open(),
            INotifyState::Closed => return Ok(&self.events[..]),
            INotifyState::Closing => self.handle_closing(),
            _ => panic!("Unexpected State {:?}", state),
        }
    }

    fn handle_open(&mut self) -> io::Result<&[Event]> {
        let fd = self.fd.expect("State != Closed");

        let mut events = Vec::<EpollEvent>::with_capacity(1);
        let mut event = EpollEvent {
            data: fd as u64,
            events: EpollEventType::EPOLLIN
        };

        let epfd = epoll::create1(0).unwrap();
        epoll::ctl(epfd, EpollControlOp::ADD, fd, &mut event).unwrap();
        events.push(event);

        loop {
            let events = epoll::wait(epfd, &mut events[..], 10).unwrap();

            match events {
                0 => { /* no new inotify events */ },
                _ => return self.available_events(),
            }

            let state = self.state();
            match self.state() {
                INotifyState::Open => { /* still open, spin */ },
                INotifyState::Closing => return self.handle_closing(),
                _ => panic!("Unexpected State {:?}", state),
            }
        }
    }

    fn handle_closing(&mut self) -> io::Result<&[Event]> {
        self.close().unwrap();
        self.events.clear();
        return Ok(&self.events[..]);
    }

    /// Returns available inotify events.
    /// If no events are available, this method will simply return a slice with
    /// zero events. If you want to wait for events to become available, call
    /// `wait_for_events`.
    pub fn available_events(&mut self) -> io::Result<&[Event]> {
        self.events.clear();

        let mut buffer = [0u8; 1024];
        let len = unsafe {
            ffi::read(
                self.fd.unwrap(),
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as size_t
            )
        };

        match len {
            0 => {
                panic!(
                    "Call to read returned 0. This should never happen and may \
                    indicate a bug in inotify-rs. For example, the buffer used \
                    to read into might be too small."
                );
            }
            -1 => {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(&self.events[..]);
                }
                else {
                    return Err(error);
                }
            },
            _ =>
                ()
        }

        let event_size = mem::size_of::<inotify_event>();

        let mut i = 0;
        while i < len {
            unsafe {
                let slice = &buffer[i as usize..];

                let event = slice.as_ptr() as *const inotify_event;

                let name = if (*event).len > 0 {
                    let name_ptr = slice
                        .as_ptr()
                        .offset(event_size as isize);

                    let name_slice_with_0 = slice::from_raw_parts(
                        name_ptr,
                        (*event).len as usize,
                    );

                    // This split ensures that the slice contains no \0 bytes, as CString
                    // doesn't like them. It will replace the slice with all bytes before the
                    // first \0 byte, or just leave the whole slice if the slice doesn't contain
                    // any \0 bytes. Using .unwrap() here is safe because .splitn() always returns
                    // at least 1 result, even if the original slice contains no instances of \0.
                    let name_slice = name_slice_with_0.splitn(2, |b| b == &0u8).next().unwrap();

                    let c_str = try!(CString::new(name_slice));

                    match String::from_utf8(c_str.as_bytes().to_vec()) {
                        Ok(string)
                            => string.to_string(),
                        Err(e) =>
                            panic!("Failed to convert C string into Rust string: {}", e)
                    }
                }
                else {
                    "".to_string()
                };

                self.events.push(Event::new(&*event, name));

                i += (event_size + (*event).len as usize) as ssize_t;
            }
        }

        Ok(&self.events[..])
    }

    // TODO do not return an Arc
    pub fn closer(&mut self) -> Arc<INotifyCloser> {
        if self.closer.is_none() {
            self.closer = Some(Arc::new(INotifyCloser {
                state: AtomicUsize::new(self.state().as_usize()),
                mutex: Mutex::new(()),
                cvar: Condvar::new(),
            }));
        }

        self.closer.as_ref().unwrap().clone()
    }

    pub fn close(&mut self) -> io::Result<()> {
        self.transition_state(|state| {
            assert!(state != INotifyState::Closed);
            let fd = self.fd.expect("State != Closed");

            let result = unsafe { ffi::close(fd) };

            match result {
                0 => (INotifyState::Closed, Ok(())),
                _ => (INotifyState::CloseFailed, Err(io::Error::last_os_error()))
            }
        }).map(|_| {
            // Success => We're definetely done with this fd.
            self.fd = None;
        })
    }

    fn transition_state<A, R>(&self, action: A) -> R
        where A: FnOnce(INotifyState) -> (INotifyState, R) {

        let state = self.state();
        match self.closer {
            Some(ref closer) => {
                let _ = closer.mutex.lock().unwrap();
                let (new_state, result) = action(state);

                if state != new_state {
                    closer.state.store(new_state.as_usize(), Ordering::Relaxed);
                    closer.cvar.notify_all();
                }

                result
            },
            None => action(state).1,
        }
    }

    fn state(&self) -> INotifyState {
        match self.closer {
            Some(ref closer) => closer.state(),
            None => match self.fd {
                Some(_) => INotifyState::Open,
                None => INotifyState::Closed,
            },
        }
    }
}

impl Drop for INotify {
    fn drop(&mut self) {
        if self.fd.is_some() {
            let _ = self.close();
        }
    }
}

impl INotifyCloser {
    pub fn close_async(&self) {
        if self.state() == INotifyState::Open {
            self.state.store(CLOSING, Ordering::Relaxed);
        }
    }

    // TODO return a result(?)
    pub fn close_sync(&self) {
        if self.state().as_usize() & (CLOSED | CLOSE_FAILED) != 0 {
            return;
        }

        let mut guard = self.mutex.lock().unwrap();
        let state = self.state();

        if state == INotifyState::Open {
            self.state.store(CLOSING, Ordering::Relaxed);
        }

        while self.state().as_usize() & (CLOSED | CLOSE_FAILED) == 0 {
            guard = self.cvar.wait(guard).unwrap();
        }
    }

    fn state(&self) -> INotifyState {
        INotifyState::from_usize(self.state.load(Ordering::Relaxed))
    }
}

impl INotifyState {
    fn from_usize(state: usize) -> INotifyState {
        match state {
            OPEN => INotifyState::Open,
            CLOSING => INotifyState::Closing,
            CLOSED => INotifyState::Closed,
            CLOSE_FAILED => INotifyState::CloseFailed,
            _ => panic!("Unexpected argument: {}", state),
        }
    }

    fn as_usize(self) -> usize {
        match self {
            INotifyState::Open => OPEN,
            INotifyState::Closing => CLOSING,
            INotifyState::Closed => CLOSED,
            INotifyState::CloseFailed => CLOSE_FAILED,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub wd    : i32,
    pub mask  : u32,
    pub cookie: u32,
    pub name  : String,
}

impl Event {
    fn new(event: &inotify_event, name: String) -> Event {
        Event {
            wd    : event.wd,
            mask  : event.mask,
            cookie: event.cookie,
            name  : name,
        }
    }

    pub fn is_access(&self) -> bool {
        return self.mask & ffi::IN_ACCESS > 0;
    }

    pub fn is_modify(&self) -> bool {
        return self.mask & ffi::IN_MODIFY > 0;
    }

    pub fn is_attrib(&self) -> bool {
        return self.mask & ffi::IN_ATTRIB > 0;
    }

    pub fn is_close_write(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_WRITE > 0;
    }

    pub fn is_close_nowrite(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_NOWRITE > 0;
    }

    pub fn is_open(&self) -> bool {
        return self.mask & ffi::IN_OPEN > 0;
    }

    pub fn is_moved_from(&self) -> bool {
        return self.mask & ffi::IN_MOVED_FROM > 0;
    }

    pub fn is_moved_to(&self) -> bool {
        return self.mask & ffi::IN_MOVED_TO > 0;
    }

    pub fn is_create(&self) -> bool {
        return self.mask & ffi::IN_CREATE > 0;
    }

    pub fn is_delete(&self) -> bool {
        return self.mask & ffi::IN_DELETE > 0;
    }

    pub fn is_delete_self(&self) -> bool {
        return self.mask & ffi::IN_DELETE_SELF > 0;
    }

    pub fn is_move_self(&self) -> bool {
        return self.mask & ffi::IN_MOVE_SELF > 0;
    }

    pub fn is_move(&self) -> bool {
        return self.mask & ffi::IN_MOVE > 0;
    }

    pub fn is_close(&self) -> bool {
        return self.mask & ffi::IN_CLOSE > 0;
    }

    pub fn is_dir(&self) -> bool {
        return self.mask & ffi::IN_ISDIR > 0;
    }

    pub fn is_unmount(&self) -> bool {
        return self.mask & ffi::IN_UNMOUNT > 0;
    }

    pub fn is_queue_overflow(&self) -> bool {
        return self.mask & ffi::IN_Q_OVERFLOW > 0;
    }

    pub fn is_ignored(&self) -> bool {
        return self.mask & ffi::IN_IGNORED > 0;
    }
}
