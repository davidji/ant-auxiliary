

use micropb::{ PbDecoder, PbEncoder, PbWrite, MessageDecode, MessageEncode};
use rtic_sync::channel::{ Receiver, Sender};
use defmt::{ debug, error };

use crate::proto::{ Request, Response };
use cobs::{ CobsDecoder, CobsEncoder, DestBufTooSmallError };

pub const MESSAGE_SIZE: usize = 64;
pub const MESSAGE_CAPACITY: usize = 4;
pub type Message = Response;
pub type ResponseSender = Sender<'static, Response, MESSAGE_CAPACITY>;
pub type ResponseReceiver = Receiver<'static, Response, MESSAGE_CAPACITY>;


pub struct CommandRequests<'a, const N: usize> {
    pub responses: ResponseSender,
    pub requests: Receiver<'a, u8, N>,
}

impl <const N: usize> CommandRequests<'_, N> {
    pub async fn receive(&mut self) -> Request {
        let mut buffer: [u8; MESSAGE_SIZE] = [0; MESSAGE_SIZE];
        loop {
            let size = self.frame(&mut buffer).await;
            debug!("message received {:x}", buffer);
            let mut request = Request::default();
            let mut pb = PbDecoder::new(buffer.as_slice());
            match request.decode(&mut pb, size) {
                Ok(()) => { return request; },
                Err(_) => {
                    error!("pb decode {}", buffer);
                }
            }
        }
    }

    async fn frame(&mut self, buffer: &mut [u8;MESSAGE_SIZE]) -> usize {
        let mut cobs = CobsDecoder::new(buffer);
        let mut optional: Option<usize> = None;
        while let None = optional {
            optional = match self.requests.recv().await {
                Ok(data) => {
                    debug!("byte received {:x}", data);
                    match cobs.feed(data) {
                        Ok(None) => None,
                        Ok(Some(size)) => Some(size),
                        Err(err) => {
                            error!("cobs: {}", err); 
                            cobs = CobsDecoder::new(buffer);
                            None
                        }
                    }
                },
                Err(err) => {
                    error!("channel: {}", err);
                    None
                 }
            }          
        }

        optional.unwrap()
    }
}


pub struct CommandResponses<'a, const N: usize> {
    pub messages: ResponseReceiver,
    pub sender: Sender<'a, u8, N>,
}

struct PbCobsEncoder<'a>(CobsEncoder<'a>);

impl <'a> PbWrite for PbCobsEncoder<'a> {
    type Error = DestBufTooSmallError;

    fn pb_write(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.0.push(data)
    }
}

impl <'a> PbCobsEncoder<'a> {
    pub fn new(out_buf: &'a mut [u8]) -> Self {
        PbCobsEncoder(CobsEncoder::new(out_buf))
    }
}

impl <const N: usize> CommandResponses<'_, N> {
    pub async fn send(&mut self) {
        loop {
            match self.messages.recv().await {
                Ok(message) => {
                    debug!("received response");
                    let mut buffer: [u8;MESSAGE_SIZE] = [0;MESSAGE_SIZE];
                    let mut cobs = PbCobsEncoder::new(&mut buffer);
                    let mut encoder = PbEncoder::new(&mut cobs);
                    match message.encode(&mut encoder) {
                        Ok(()) => {
                            let size = cobs.0.finalize();
                            debug!("sending response {:x}", buffer[0..size]);
                            for data in buffer[0..size].iter() {
                                self.sender.send(*data).await.unwrap();
                            }
                            self.sender.send(0).await.unwrap();
                        },
                        Err(DestBufTooSmallError) => error!("destination buffer too small")
                    }
                },
                Err(e) => { 
                    error!("error reading from shell message channel: {}", e);
                    return;
                }
            }
        }

    }
}

pub struct TaskResponses<T> {
    pub task: T,
    pub responses: ResponseSender
}
