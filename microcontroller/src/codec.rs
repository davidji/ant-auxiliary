


use core::marker::PhantomData;

use micropb::{ MessageDecode, MessageEncode, PbDecoder, PbEncoder, PbWrite };
use defmt::{ debug, error };
use cobs::{ CobsDecoder, CobsEncoder, DestBufTooSmallError };

use crate::stream::{ ByteSupplier , ByteConsumer, Supplier, Consumer };

pub struct Decoder<I, O, const BN: usize> {
    input: I,
    buffer : [u8; BN],
    target: PhantomData<O>,
}

impl <I: ByteSupplier, O, const BN: usize> Decoder<I, O, BN> {
    pub fn new(requests: I) -> Self {
        Decoder { input: requests, buffer: [0; BN], target: PhantomData::default() }
    }

    async fn frame(&mut self) -> usize {
        let mut cobs = CobsDecoder::new(&mut self.buffer);
        let mut optional: Option<usize> = None;
        while let None = optional { 
            optional = match self.input.next().await {
                Some(byte) => {
                    debug!("byte received {:x}", byte);
                    match cobs.feed(byte) {
                        Ok(None) => None,
                        Ok(Some(size)) => Some(size),
                        Err(err) => {
                            error!("cobs: {}", err); 
                            cobs = CobsDecoder::new(&mut self.buffer);
                            None
                        }
                    }
                },
                None => Some(0),
            }          
        }

        optional.unwrap()
    }
}

impl <I: ByteSupplier, O, const BN: usize> Supplier for Decoder<I, O, BN>
where O: MessageEncode + MessageDecode + Default {
    type Item = O;

    async fn next(&mut self) -> Option<Self::Item> {
        loop {
            let size = self.frame().await;
            debug!("message received {:x}", self.buffer[0..size]);
            let mut request = Self::Item::default();
            let mut pb = PbDecoder::new(self.buffer.as_slice());
            match request.decode(&mut pb, size) {
                Ok(()) => { return Some(request); },
                Err(_) => {
                    error!("pb decode {}", self.buffer);
                }
            }
        }
    }
}

pub struct Encoder<I, O, const BN: usize> {
    output: O,
    input: PhantomData<I>,
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

impl <I, O, const BN: usize> Encoder<I, O, BN> {
    pub fn new(output: O) -> Self {
        Encoder { output, input: PhantomData::default() }
    }
}

impl <I, O: ByteConsumer, const BN: usize> Consumer for Encoder<I, O, BN> 
where I: MessageEncode {
    type Item = I;
    type Error = O::Error;

    async fn accept(&mut self, message: I) -> Result<(), O::Error> {
        let mut buffer: [u8;BN] = [0;BN];
        let mut cobs = PbCobsEncoder::new(&mut buffer);
        let mut encoder = PbEncoder::new(&mut cobs);
        match message.encode(&mut encoder) {
            Ok(()) => {
                let size = cobs.0.finalize();
                debug!("sending response {:x}", buffer[0..size]);
                for data in buffer[0..size].iter() {
                    self.output.accept(*data).await?
                }
                self.output.accept(0).await
            },
            Err(DestBufTooSmallError) => panic!("destination buffer too small")
        }
    }
}

