
use crate::proto::{ Request, Response };
use rtic_sync::channel::{ Receiver, ReceiveError, Sender};

pub const MESSAGE_CAPACITY: usize = 4;
pub type ResponseSender = Sender<'static, Response, MESSAGE_CAPACITY>;
pub type ResponseReceiver = Receiver<'static, Response, MESSAGE_CAPACITY>;

pub struct TaskResponses<T> {
    pub task: T,
    pub responses: ResponseSender
}
