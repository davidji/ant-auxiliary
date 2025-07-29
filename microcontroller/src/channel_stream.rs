
use rtic_sync::channel::{ NoReceiver, Receiver, Sender };
use crate::stream::{ ByteConsumer, ByteSupplier, Supplier, Consumer };

pub struct ChannelSupplier <'a, T, const N: usize> (Receiver<'a, T, N>);

impl <'a, T, const N: usize>  ChannelSupplier<'a, T, N> {
    pub fn new(receiver: Receiver<'a, T, N>) -> Self {
        ChannelSupplier(receiver)
    }
}

impl <'a, T, const N: usize> Supplier for ChannelSupplier<'a, T, N> {
    type Item = T;

    async fn next(&mut self) -> Option<T> {
        match self.0.recv().await {
            Ok(byte) => Some(byte),
            Err(_) => None,
        }
    }
}

impl <'a, const N: usize> ByteSupplier for ChannelSupplier<'a, u8, N> {

}

pub struct ChannelConsumer<'a, T, const N: usize>(Sender<'a, T, N>);

impl <'a, T, const N: usize> ChannelConsumer<'a, T, N> {
    pub fn new(sender: Sender<'a, T, N>) -> Self {
        ChannelConsumer(sender)
    }
}

impl <'a, T, const N: usize> Consumer for ChannelConsumer<'a, T, N> {
    type Item = T;
    type Error = NoReceiver<T>;

    async fn accept(&mut self, item: T) -> Result<(), Self::Error> {
        self.0.send(item).await
    }
}

impl <'a, const N: usize> ByteConsumer for ChannelConsumer<'a, u8, N> {

}
