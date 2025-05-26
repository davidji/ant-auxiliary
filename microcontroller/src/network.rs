use core::future::poll_fn;

use futures::task::Poll;

use defmt::error;

use stm32f4xx_hal::otg_fs::{ UsbBus, USB };

use usbd_ethernet::{ Ethernet, DeviceState };
use usb_device::UsbError;

use smoltcp::{
    iface::{self, Interface, SocketHandle, SocketSet },
    socket::tcp,
    time::Instant,
    wire::{ EthernetAddress, Ipv4Address, Ipv4Cidr },
};

use rtic_sync::channel::{ Channel, ReceiveError, Receiver, Sender};

pub const IP_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 0, 1);
const DEVICE_MAC_ADDR: [u8; 6] = [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];
pub const MTU: u16 = 64;

pub fn usb_ethernet<'a>(
    usb_alloc: &'a usb_device::bus::UsbBusAllocator<UsbBus<USB>>,
    in_buffer: &'a mut [u8; 2048],
    out_buffer: &'a mut [u8; 2048]) ->  Ethernet<'a, UsbBus<USB>> {

    Ethernet::new(
        usb_alloc,
        DEVICE_MAC_ADDR,
        MTU,
        in_buffer,
        out_buffer)
}

pub fn interface<'a>(ethernet: &mut Ethernet<'a, UsbBus<USB>>) -> Interface {
    let mut interface_config = iface::Config::new(EthernetAddress(DEVICE_MAC_ADDR).into());
    interface_config.random_seed = 0;

    let mut interface = Interface::new(
        interface_config,
        ethernet,
        smoltcp::time::Instant::ZERO);

    interface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(Ipv4Cidr::new(IP_ADDRESS, 0).into())
            .unwrap();
    });

    interface
}

pub struct NetworkStack<'a> {
    pub ethernet: Ethernet<'a, UsbBus<USB>>,
    pub interface: Interface,
    pub sockets: SocketSet<'a>,
}

pub struct RecvChannel<'a, const N: usize> {
    pub handle: SocketHandle,
    pub sender: Sender<'a, u8, N>
}

impl <const N: usize> RecvChannel<'_, N> {
    pub fn try_recv(&mut self,  sockets: &mut SocketSet<'_>) {
        let socket: &mut tcp::Socket = sockets.get_mut(self.handle);
        let mut buf = [0u8; 1];
        while socket.can_recv() && !self.sender.is_full() {
            match socket.recv_slice(&mut buf) {
                Ok(_) => self.sender.try_send(buf[0]).unwrap(),
                Err(_) => panic!("Error writing to USB serial"),
            };
        }
    }
}
pub struct SendChannel<'a, const N: usize> {
    pub handle: SocketHandle,
    pub receiver: Receiver<'a, u8, N>
}

impl <const N: usize> SendChannel<'_, N> {
    pub async fn send(&mut self,  sockets: &mut SocketSet<'_>) -> Result<bool, ReceiveError> {
        poll_fn(|cx| {

            match self.try_send(sockets) {
                Ok(false) => Poll::Pending,
                Ok(true) => Poll::Ready(Ok(true)),
                Err(err) => return Poll::Ready(Err(err)),
            }
        }).await
    }

    pub fn try_send(&mut self, sockets: &mut SocketSet<'_>) -> Result<bool, ReceiveError> {
        let socket:&mut tcp::Socket = sockets.get_mut(self.handle);
        let mut count: usize = 0;
        while socket.can_send() {
            match self.receiver.try_recv() {
                Ok(data) => {
                    socket.send_slice(&[data]).ok();
                    count += 1;
                },
                Err(ReceiveError::Empty) => { 
                    break; 
                },
                Err(err) => {
                    return Err(err);
                }
            }
        }
        Ok(count != 0)
    }   
}

pub struct NetworkChannelStorage<const N: usize> {
    pub sender: Channel<u8, N>,
    pub receiver: Channel<u8, N>,
    pub tx_storage: [u8; N],
    pub rx_storage: [u8; N],
}

impl  <const N: usize> NetworkChannelStorage<N> {
    

    pub const fn new() -> Self {
        Self {
            sender: Channel::new(),
            receiver: Channel::new(),
            tx_storage: [0x0; N],
            rx_storage: [0x0; N],
        }
    }
}

pub struct NetworkEndpoint<'a, const N: usize> {
    pub send: SendChannel<'a, N>,
    pub recv: RecvChannel<'a, N>,
}

pub struct ApplicationEndpoint<'a, const N: usize> {
    pub send: Sender<'a, u8, N>,
    pub recv: Receiver<'a, u8, N>,
}

pub struct NetworkChannel<'a, const N: usize> {
    pub net: NetworkEndpoint<'a, N>,
    pub app: ApplicationEndpoint<'a, N>
}

impl <'a> NetworkStack<'a>  {
    pub fn connect(&mut self)  {
        if self.ethernet.state() == DeviceState::Disconnected {
            if self.ethernet.connection_speed().is_none() {
                // 1000 Kps upload and download
                match self.ethernet.set_connection_speed(1_000_000, 1_000_000) {
                    Ok(()) | Err(UsbError::WouldBlock) => {}
                    Err(e) => error!("Failed to set connection speed: {}", e),
                }
            } else if self.ethernet.state() == DeviceState::Disconnected {
                match self.ethernet.connect() {
                    Ok(()) | Err(UsbError::WouldBlock) => {}
                    Err(e) => error!("Failed to connect: {}", e),
                }
            }
        }
    }

    pub fn connected(&mut self) -> bool {
        self.ethernet.state() == DeviceState::Connected
    }
    
    pub fn try_send<const N: usize>(&mut self, now: Instant, channels: &mut [SendChannel<N>]) {
        if self.connected() {
            let mut data = false;
            for channel in channels {
                data |= match channel.try_send(&mut self.sockets) {
                    Ok(sent) => sent,
                    Err(_) => panic!("Error reading from channel reciever")
                }
            }

            if data {
                self.interface.poll_egress(now, &mut self.ethernet, &mut self.sockets);
            }
        } else {
            self.connect();
        }
    }

    pub fn try_recv<const N: usize>(&mut self, now: Instant, channels: &mut [RecvChannel<N>]) {
        let data = match self.interface.poll(now, &mut self.ethernet, &mut self.sockets) {
            iface::PollResult::SocketStateChanged => true,
            iface::PollResult::None => false
        };

        if data {
            for channel in channels {
                channel.try_recv(&mut self.sockets);
            }
        }
    }
   
    pub fn channel<const N:usize>(&mut self, storage: &'a mut NetworkChannelStorage<N>) -> NetworkChannel<'a, N> {
        let rx_buffer = tcp::SocketBuffer::new(&mut storage.rx_storage[..]);
        let tx_buffer = tcp::SocketBuffer::new(&mut storage.tx_storage[..]);

        let socket = tcp::Socket::new(rx_buffer, tx_buffer);
        let handle = self.sockets.add(socket);
    
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        socket.listen(crate::SOCKET_ADDRESS).ok();
      
        let (net_send, app_recv) = storage.receiver.split();
        let (app_send, net_recv) = storage.sender.split();

        NetworkChannel {
            net: NetworkEndpoint { 
                send: SendChannel { handle: handle, receiver: net_recv },
                recv: RecvChannel { handle: handle, sender: net_send },
            },
            app: ApplicationEndpoint { send: app_send, recv: app_recv }
        }
    }
}
