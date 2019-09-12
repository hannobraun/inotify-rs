use std::{
    io,
    ops::Deref,
    sync::Arc,
};

#[cfg(any(feature = "async-await", feature = "stream"))]
use mio::{
    Evented,
    unix::EventedFd,
};

use crate::fd_guard::FdGuard;
use crate::util::read_into_buffer;

#[derive(Clone, Debug, PartialEq)]
pub struct EventedFdGuard(pub Arc<FdGuard>);

#[cfg(any(feature = "async-await", feature = "stream"))]
impl Evented for EventedFdGuard {
    #[inline]
    fn register(&self,
                poll: &mio::Poll,
                token: mio::Token,
                interest: mio::Ready,
                opts: mio::PollOpt)
                -> io::Result<()>
    {
        EventedFd(&(self.fd)).register(poll, token, interest, opts)
    }

    #[inline]
    fn reregister(&self,
                  poll: &mio::Poll,
                  token: mio::Token,
                  interest: mio::Ready,
                  opts: mio::PollOpt)
                  -> io::Result<()>
    {
        EventedFd(&(self.fd)).reregister(poll, token, interest, opts)
    }

    #[inline]
    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        EventedFd(&self.fd).deregister(poll)
    }
}

impl io::Read for EventedFdGuard {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match read_into_buffer(self.fd, buf) {
            i if i >= 0 => Ok(i as usize),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Deref for EventedFdGuard {
    type Target = Arc<FdGuard>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
