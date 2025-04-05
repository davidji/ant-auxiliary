use core::task::Context;
use core::pin::Pin;

use rtic_sync::channel::*;
use usbd_serial::embedded_io::{ Read, ReadReady, Write, WriteReady};
use futures::{
    future::Future,
    task::Poll
};

pub struct ReadFuture<'a, R> {
    reader: &'a mut R, 
}

impl <'a, R> Future for ReadFuture<'a, R>
where R: Read + ReadReady {
    type Output = Result<u8, R::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.reader.read_ready() {
            Ok(true) => {
                let mut buf = [0u8; 1];
                match self.reader.read(&mut buf) {
                    Ok(_) => Poll::Ready(Ok(buf[0])),
                    Err(e) => Poll::Ready(Err(e))
                }
            },
            Ok(false) => Poll::Pending,
            Err(e)=> Poll::Ready(Err(e)),
        }
    }
}

pub struct WriteFuture<'a, W> {
    writer: &'a mut W
}


pub struct ReadRelay<'a, R, const N: usize> {
    reader: &'a mut R, 
    sender: &'a mut Sender<'a,u8,N>
}

enum ReadRelayError<R>
where R: Read + ReadReady {
    ReadError(R::Error),
    SendError(TrySendError<u8>)
}

impl <'a, R, const N: usize> Future for ReadRelay<'a, R, N>
where R: Read + ReadReady {

    type Output = Result<(),ReadRelayError<R>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match (self.reader.read_ready(), self.sender.is_full()) {
            (Ok(true), false) => {
                let mut buf = [0u8; 1];
                match self.reader.read(&mut buf) {
                    Ok(_) => match self.sender.try_send(buf[0]) {
                        Ok(_) => Poll::Ready(Ok(())),
                        Err(e) => Poll::Ready(Err(ReadRelayError::SendError(e)))
                    },
                    Err(e) => Poll::Ready(Err(ReadRelayError::ReadError(e))),
                }
            },
            (Ok(_), _) => Poll::Pending,
            (Err(e), _) => Poll::Ready(Err(ReadRelayError::ReadError(e))),
        }
    }
}

pub struct WriteRelay<'a, W, const N: usize> {
    writer: &'a mut W, 
    receiver: &'a mut Receiver<'a,u8,N>
}

enum WriteRelayError<W>
where W: Write + WriteReady {
    WriteError(W::Error),
    SendError(TryReceiveError<u8>)
}

impl <'a, W, const N: usize> Future for WriteRelay<'a, W, N>
where W: Write + WriteReady {
    type Output = Result<(),WriteRelayError<W>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match (self.writer.write_ready(), self.receiver.is_empty()) {
            (Ok(true), false) => {
                match self.receiver.poll(cx) {
                    
                }
            }
        }
    }
}